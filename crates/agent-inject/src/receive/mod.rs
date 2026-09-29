//! One upload: read the header, check the secret, stream the body to disk,
//! answer with a [`Response`].
//!
//! Generic over the stream halves so the whole protocol runs in unit tests
//! over an in-memory pipe; the network handler passes the QUIC stream halves.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use agent_inject_proto::framing::{REQUEST_PREFIX_LEN, remaining_header_len};
use agent_inject_proto::{RequestHeader, Response, SECRET_LEN, Status, UPLOAD_ID_LEN, ct_eq};
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
}

impl ReceiveCtx {
    pub(crate) fn new(dir: PathBuf, secret: [u8; SECRET_LEN], max_size: Option<u64>) -> Self {
        Self {
            dir,
            secret,
            max_size,
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

    fn remember(&self, upload_id: [u8; UPLOAD_ID_LEN], path: PathBuf) {
        self.seen
            .lock()
            .expect("seen map poisoned")
            .insert(upload_id, path);
    }
}

/// Receive one upload from `recv` and answer on `send`.
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
    let outcome = match read_header(&mut recv).await? {
        Err(status) => Err((status, "malformed header".to_owned())),
        Ok(header) => handle(ctx, &header, &mut recv).await?,
    };
    let (response, result) = match outcome {
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
        Err((status, message)) => (Response { status, message }, Outcome::Refused(status)),
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
    let name = name::sanitize(&header.name);
    let path = temp
        .finalize(&ctx.dir, &name)
        .await
        .context("save upload")?;
    ctx.remember(header.upload_id, path.clone());
    Ok(Ok((path, true)))
}

fn truncated() -> (Status, String) {
    (
        Status::Truncated,
        "body did not match the declared size".to_owned(),
    )
}

/// Read the whole header, or the status to refuse it with.
async fn read_header<R: AsyncRead + Unpin>(
    recv: &mut R,
) -> Result<std::result::Result<RequestHeader, Status>> {
    let mut header = vec![0u8; REQUEST_PREFIX_LEN];
    recv.read_exact(&mut header)
        .await
        .context("read request prefix")?;
    let Ok(rest) = remaining_header_len(&header) else {
        return Ok(Err(Status::BadName));
    };
    header.resize(REQUEST_PREFIX_LEN + rest, 0);
    recv.read_exact(&mut header[REQUEST_PREFIX_LEN..])
        .await
        .context("read request header")?;
    Ok(RequestHeader::decode(&header).map_err(|_| Status::BadName))
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

    use agent_inject_proto::{RequestHeader, Response, Status};
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
        ReceiveCtx::new(dir.to_owned(), SECRET, Some(1024 * 1024))
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
        let ctx = ReceiveCtx::new(dir.path().to_owned(), SECRET, Some(4));
        let (outcome, _) = upload(&ctx, request("a.txt", 1, 5, b"12345", SECRET)).await;
        assert_eq!(outcome, Outcome::Refused(Status::TooLarge));
        assert!(entries(dir.path()).is_empty());
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
