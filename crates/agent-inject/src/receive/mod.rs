//! One upload: read the header, check the secret, stream the body to disk,
//! answer with a [`Response`].
//!
//! Generic over the stream halves so the whole protocol runs in unit tests
//! over an in-memory pipe; the network handler passes the QUIC stream halves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use agent_inject_proto::framing::{
    OP_PREFIX_LEN, Op, REQUEST_PREFIX_LEN, decode_op, remaining_header_len,
};
use agent_inject_proto::{
    Accept, RequestHeader, Response, SECRET_LEN, Status, UPLOAD_ID_LEN, ct_eq,
};
use anyhow::{Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter};

use self::store::TempFile;

pub(crate) mod name;
pub(crate) mod store;

const CHUNK: usize = 64 * 1024;

/// What every upload in one session shares.
#[derive(Debug)]
pub(crate) struct ReceiveCtx {
    dir: PathBuf,
    secret: [u8; SECRET_LEN],
    max_size: Option<u64>,
    accept: Accept,
    /// Upload id → final path, so a retry after a lost ack gets the same
    /// answer instead of a second copy.
    seen: Mutex<HashMap<[u8; UPLOAD_ID_LEN], PathBuf>>,
}

/// How one upload ended, when the stream itself did not fail.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Saved(PathBuf),
    AlreadySaved(PathBuf),
    Refused(Status),
    /// The sender has nothing more to send; the session is over.
    Done,
}

impl ReceiveCtx {
    pub(crate) fn new(
        dir: PathBuf,
        secret: [u8; SECRET_LEN],
        max_size: Option<u64>,
        accept: Accept,
    ) -> Self {
        Self {
            dir,
            secret,
            max_size,
            accept,
            seen: Mutex::new(HashMap::new()),
        }
    }

    fn already_saved(&self, upload_id: &[u8; UPLOAD_ID_LEN]) -> Option<PathBuf> {
        self.seen
            .lock()
            .expect("seen map poisoned")
            .get(upload_id)
            .cloned()
    }

    /// Files saved this session: each upload id is one saved file.
    fn saved_count(&self) -> usize {
        self.seen.lock().expect("seen map poisoned").len()
    }

    fn remember(&self, upload_id: [u8; UPLOAD_ID_LEN], path: PathBuf) {
        self.seen
            .lock()
            .expect("seen map poisoned")
            .insert(upload_id, path);
    }
}

/// Receive one request from `recv` and answer on `send`: an upload, or the
/// done signal that ends the session.
///
/// # Errors
/// The stream failed (reset, closed early mid-header, or a write error while
/// answering). A refused or truncated upload is not an error: it is answered
/// and reported as [`Outcome::Refused`].
pub(crate) async fn receive_core<R, W>(
    ctx: &ReceiveCtx,
    mut recv: R,
    mut send: W,
) -> Result<Outcome>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let refuse = |status: Status, message: &str| {
        (
            Response {
                status,
                message: message.to_owned(),
            },
            Outcome::Refused(status),
        )
    };
    let (response, result) = match read_request(&mut recv).await? {
        Err(status) => refuse(status, "malformed request"),
        Ok(Request::Done(secret)) if ct_eq(&secret, &ctx.secret) => (
            Response {
                status: Status::Ok,
                message: ctx.saved_count().to_string(),
            },
            Outcome::Done,
        ),
        Ok(Request::Done(_)) => refuse(Status::Unauthorized, "wrong secret"),
        Ok(Request::Upload(header)) => match handle(ctx, &header, &mut recv).await? {
            Ok((path, fresh)) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let result = if fresh {
                    Outcome::Saved(path)
                } else {
                    Outcome::AlreadySaved(path)
                };
                (
                    Response {
                        status: Status::Ok,
                        message: name,
                    },
                    result,
                )
            }
            Err((status, message)) => refuse(status, &message),
        },
    };
    send.write_all(&response.encode())
        .await
        .context("write response")?;
    send.shutdown().await.context("finish response")?;
    Ok(result)
}

type Handled = std::result::Result<(PathBuf, bool), (Status, String)>;

async fn handle<R: AsyncRead + Unpin>(
    ctx: &ReceiveCtx,
    header: &RequestHeader,
    recv: &mut R,
) -> Result<Handled> {
    if !ct_eq(&header.secret, &ctx.secret) {
        return Ok(Err((Status::Unauthorized, "wrong secret".to_owned())));
    }
    if let Some(max) = ctx.max_size
        && header.size > max
    {
        return Ok(Err((
            Status::TooLarge,
            format!("{} bytes is over the {max}-byte limit", header.size),
        )));
    }
    let name = name::sanitize(&header.name);
    if !accepts(ctx.accept, &name) {
        return Ok(Err((
            Status::NotAccepted,
            "this session takes photos only".to_owned(),
        )));
    }
    if let Some(path) = ctx.already_saved(&header.upload_id) {
        return Ok(match drain(recv, header.size).await? {
            BodyEnd::Exact => Ok((path, false)),
            BodyEnd::Short | BodyEnd::Long => Err(truncated()),
        });
    }

    let mut temp = TempFile::create(&ctx.dir)
        .await
        .context("create temp file")?;
    let mut writer = BufWriter::with_capacity(CHUNK, temp.file());
    let end = copy_body(recv, &mut writer, header.size).await?;
    writer.flush().await.context("flush upload")?;
    if !matches!(end, BodyEnd::Exact) {
        return Ok(Err(truncated()));
    }
    let path = temp
        .finalize(&ctx.dir, &name)
        .await
        .context("save upload")?;
    ctx.remember(header.upload_id, path.clone());
    Ok(Ok((path, true)))
}

/// The header carries only a name, so the extension decides. HEIC is on the
/// list because iPhone pickers hand over HEIC originals.
fn accepts(accept: Accept, name: &str) -> bool {
    const IMAGE_EXTENSIONS: &[&str] =
        &["jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "avif"];
    match accept {
        Accept::Any => true,
        Accept::Images => std::path::Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                IMAGE_EXTENSIONS
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            }),
    }
}

fn truncated() -> (Status, String) {
    (
        Status::Truncated,
        "body did not match the declared size".to_owned(),
    )
}

enum Request {
    Upload(RequestHeader),
    Done([u8; SECRET_LEN]),
}

/// Read the whole request header, or the status to refuse it with.
async fn read_request<R: AsyncRead + Unpin>(
    recv: &mut R,
) -> Result<std::result::Result<Request, Status>> {
    let mut header = vec![0u8; OP_PREFIX_LEN];
    recv.read_exact(&mut header)
        .await
        .context("read request prefix")?;
    let secret = match decode_op(&header) {
        Err(_) => return Ok(Err(Status::BadName)),
        Ok((secret, Op::Done)) => return Ok(Ok(Request::Done(secret))),
        Ok((secret, Op::Upload)) => secret,
    };
    header.resize(REQUEST_PREFIX_LEN, 0);
    recv.read_exact(&mut header[OP_PREFIX_LEN..])
        .await
        .context("read upload prefix")?;
    let Ok(rest) = remaining_header_len(&header) else {
        return Ok(Err(Status::BadName));
    };
    header.resize(REQUEST_PREFIX_LEN + rest, 0);
    recv.read_exact(&mut header[REQUEST_PREFIX_LEN..])
        .await
        .context("read request header")?;
    debug_assert_eq!(header[..SECRET_LEN], secret);
    Ok(RequestHeader::decode(&header)
        .map(Request::Upload)
        .map_err(|_| Status::BadName))
}

enum BodyEnd {
    Exact,
    Short,
    Long,
}

/// Copy exactly `size` bytes, then make sure the sender stopped there.
async fn copy_body<R, W>(recv: &mut R, writer: &mut W, size: u64) -> Result<BodyEnd>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    let mut remaining = size;
    while remaining > 0 {
        let want = usize::try_from(remaining.min(CHUNK as u64)).expect("bounded by CHUNK");
        let read = recv.read(&mut buf[..want]).await.context("read body")?;
        if read == 0 {
            return Ok(BodyEnd::Short);
        }
        writer.write_all(&buf[..read]).await.context("write body")?;
        remaining -= read as u64;
    }
    let extra = recv.read(&mut buf[..1]).await.context("read body end")?;
    Ok(if extra == 0 {
        BodyEnd::Exact
    } else {
        BodyEnd::Long
    })
}

async fn drain<R: AsyncRead + Unpin>(recv: &mut R, size: u64) -> Result<BodyEnd> {
    copy_body(recv, &mut tokio::io::sink(), size).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use agent_inject_proto::framing::encode_done;
    use agent_inject_proto::{Accept, RequestHeader, Response, Status};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{Outcome, ReceiveCtx, receive_core};

    const SECRET: [u8; 32] = [1u8; 32];

    fn request(name: &str, upload_id: u8, declared: u64, body: &[u8], secret: [u8; 32]) -> Vec<u8> {
        let mut bytes = RequestHeader {
            secret,
            upload_id: [upload_id; 16],
            name: name.to_owned(),
            size: declared,
        }
        .encode()
        .unwrap();
        bytes.extend_from_slice(body);
        bytes
    }

    /// Run one upload over an in-memory pipe and return the outcome and the
    /// decoded response.
    async fn upload(ctx: &ReceiveCtx, bytes: Vec<u8>) -> (Outcome, Response) {
        let (mut client, server) = tokio::io::duplex(256 * 1024);
        let (server_recv, server_send) = tokio::io::split(server);
        let writer = tokio::spawn(async move {
            client.write_all(&bytes).await.unwrap();
            client.shutdown().await.unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            response
        });
        let outcome = receive_core(ctx, server_recv, server_send).await.unwrap();
        let response = Response::decode(&writer.await.unwrap()).unwrap();
        (outcome, response)
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn ctx(dir: &Path) -> ReceiveCtx {
        ReceiveCtx::new(dir.to_owned(), SECRET, Some(1024 * 1024), Accept::Any)
    }

    fn images_ctx(dir: &Path) -> ReceiveCtx {
        ReceiveCtx::new(dir.to_owned(), SECRET, Some(1024 * 1024), Accept::Images)
    }

    #[tokio::test]
    async fn saves_the_body_under_the_sanitized_name() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let body = vec![42u8; 200_000];
        let (outcome, response) =
            upload(&ctx, request("../x.bin", 1, 200_000, &body, SECRET)).await;
        assert_eq!(outcome, Outcome::Saved(dir.path().join("x.bin")));
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.message, "x.bin");
        assert_eq!(std::fs::read(dir.path().join("x.bin")).unwrap(), body);
        assert_eq!(entries(dir.path()), ["x.bin"]);
    }

    #[tokio::test]
    async fn zero_byte_file_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (outcome, _) = upload(&ctx(dir.path()), request("empty", 1, 0, &[], SECRET)).await;
        assert_eq!(outcome, Outcome::Saved(dir.path().join("empty")));
        assert!(std::fs::read(dir.path().join("empty")).unwrap().is_empty());
    }

    #[tokio::test]
    async fn short_and_long_bodies_are_refused_and_leave_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        for (declared, body) in [(10u64, &b"short"[..]), (3, &b"too long"[..])] {
            let (outcome, response) =
                upload(&ctx, request("a.txt", 1, declared, body, SECRET)).await;
            assert_eq!(outcome, Outcome::Refused(Status::Truncated));
            assert_eq!(response.status, Status::Truncated);
        }
        assert!(entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn wrong_secret_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (outcome, response) =
            upload(&ctx(dir.path()), request("a.txt", 1, 1, b"x", [2u8; 32])).await;
        assert_eq!(outcome, Outcome::Refused(Status::Unauthorized));
        assert_eq!(response.status, Status::Unauthorized);
        assert!(entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn oversize_upload_is_refused_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ReceiveCtx::new(dir.path().to_owned(), SECRET, Some(4), Accept::Any);
        let (outcome, _) = upload(&ctx, request("a.txt", 1, 5, b"12345", SECRET)).await;
        assert_eq!(outcome, Outcome::Refused(Status::TooLarge));
        assert!(entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn an_images_session_refuses_other_files_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let (outcome, response) = upload(
            &images_ctx(dir.path()),
            request("notes.pdf", 1, 2, b"hi", SECRET),
        )
        .await;
        assert_eq!(outcome, Outcome::Refused(Status::NotAccepted));
        assert_eq!(response.status, Status::NotAccepted);
        assert!(entries(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn an_images_session_takes_photos_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = images_ctx(dir.path());
        for (id, name) in [(1u8, "a.jpg"), (2, "IMG_0001.HEIC"), (3, "b.png")] {
            let (outcome, _) = upload(&ctx, request(name, id, 2, b"hi", SECRET)).await;
            assert_eq!(outcome, Outcome::Saved(dir.path().join(name)));
        }
    }

    #[tokio::test]
    async fn an_any_session_takes_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let (outcome, _) =
            upload(&ctx(dir.path()), request("notes.pdf", 1, 2, b"hi", SECRET)).await;
        assert_eq!(outcome, Outcome::Saved(dir.path().join("notes.pdf")));
    }

    #[tokio::test]
    async fn retry_with_the_same_id_does_not_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let first = upload(&ctx, request("a.txt", 7, 2, b"hi", SECRET)).await;
        let retry = upload(&ctx, request("a.txt", 7, 2, b"hi", SECRET)).await;
        let other = upload(&ctx, request("a.txt", 8, 2, b"yo", SECRET)).await;
        assert_eq!(first.0, Outcome::Saved(dir.path().join("a.txt")));
        assert_eq!(retry.0, Outcome::AlreadySaved(dir.path().join("a.txt")));
        assert_eq!(retry.1.message, "a.txt");
        assert_eq!(other.0, Outcome::Saved(dir.path().join("a-2.txt")));
        assert_eq!(entries(dir.path()), ["a-2.txt", "a.txt"]);
    }

    #[tokio::test]
    async fn done_with_the_right_secret_ends_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        upload(&ctx, request("a.txt", 1, 1, b"x", SECRET)).await;
        let (outcome, response) = upload(&ctx, encode_done(&SECRET)).await;
        assert_eq!(outcome, Outcome::Done);
        assert_eq!(response.status, Status::Ok);
        assert_eq!(response.message, "1");
    }

    #[tokio::test]
    async fn done_with_a_wrong_secret_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (outcome, response) = upload(&ctx(dir.path()), encode_done(&[2u8; 32])).await;
        assert_eq!(outcome, Outcome::Refused(Status::Unauthorized));
        assert_eq!(response.status, Status::Unauthorized);
    }

    #[tokio::test]
    async fn stream_closed_mid_header_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let (mut client, server) = tokio::io::duplex(1024);
        let (server_recv, server_send) = tokio::io::split(server);
        client.write_all(&[0u8; 10]).await.unwrap();
        client.shutdown().await.unwrap();
        assert!(receive_core(&ctx, server_recv, server_send).await.is_err());
        assert!(entries(dir.path()).is_empty());
    }
}
