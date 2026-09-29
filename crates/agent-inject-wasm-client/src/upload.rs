//! One upload: the header, then the file in slices, then the answer.
//!
//! Slices rather than `Blob.stream()`: they give exact offsets for progress
//! and keep at most one chunk in memory, which matters for a video on a
//! phone.

use agent_inject_proto::{RequestHeader, Response, SECRET_LEN, Status, UPLOAD_ID_LEN};
use fofoca::iroh::endpoint::Connection;
use js_sys::Uint8Array;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::err;

/// Bytes read from the file per write. Big enough to keep the stream busy,
/// small enough that a phone never holds much of the file at once.
const CHUNK: u64 = 256 * 1024;

/// Most a response can be: status, length, and a capped message.
const MAX_RESPONSE_BYTES: usize = 3 + agent_inject_proto::framing::MAX_MESSAGE_BYTES;

pub(crate) async fn send(
    conn: &Connection,
    secret: [u8; SECRET_LEN],
    upload_id: [u8; UPLOAD_ID_LEN],
    name: String,
    blob: &web_sys::Blob,
    on_progress: Option<&js_sys::Function>,
) -> Result<String, JsValue> {
    let total = blob_size(blob);
    let header = RequestHeader {
        secret,
        upload_id,
        name,
        size: total,
    }
    .encode()
    .map_err(|error| err("bad_name", &error))?;

    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|error| err("open upload stream", &error))?;
    send.write_all(&header)
        .await
        .map_err(|error| err("send header", &error))?;

    // A write error here usually means the receiver refused and stopped our
    // stream; its response says why, so read it before reporting the write.
    let mut write_error = None;
    for (start, end) in chunk_ranges(total, CHUNK) {
        let bytes = read_slice(blob, start, end).await?;
        if let Err(error) = send.write_all(&bytes).await {
            write_error = Some(err("send body", &error));
            break;
        }
        if let Some(callback) = on_progress {
            let _ = callback.call2(
                &JsValue::NULL,
                &JsValue::from_f64(to_f64(end)),
                &JsValue::from_f64(to_f64(total)),
            );
        }
    }
    let _ = send.finish();

    let raw = match recv.read_to_end(MAX_RESPONSE_BYTES).await {
        Ok(raw) => raw,
        Err(error) => return Err(write_error.unwrap_or_else(|| err("read response", &error))),
    };
    let response = Response::decode(&raw).map_err(|error| err("decode response", &error))?;
    match response.status {
        Status::Ok => Ok(response.message),
        status @ (Status::Unauthorized
        | Status::BadName
        | Status::Truncated
        | Status::Io
        | Status::TooLarge) => Err(JsValue::from_str(&format!(
            "{}: {}",
            status.label(),
            response.message
        ))),
    }
}

/// Tell the receiver the sender has nothing more to send. Resolves with how
/// many files the session saved.
pub(crate) async fn send_done(
    conn: &Connection,
    secret: &[u8; SECRET_LEN],
) -> Result<u32, JsValue> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|error| err("open done stream", &error))?;
    send.write_all(&agent_inject_proto::framing::encode_done(secret))
        .await
        .map_err(|error| err("send done", &error))?;
    let _ = send.finish();
    let raw = recv
        .read_to_end(MAX_RESPONSE_BYTES)
        .await
        .map_err(|error| err("read done response", &error))?;
    let response = Response::decode(&raw).map_err(|error| err("decode response", &error))?;
    if response.status != Status::Ok {
        return Err(JsValue::from_str(&format!(
            "{}: {}",
            response.status.label(),
            response.message
        )));
    }
    response
        .message
        .parse()
        .map_err(|error| err("done response count", &error))
}

/// `[start, end)` byte ranges covering `total` in steps of `chunk`. Empty for
/// an empty file.
pub(crate) fn chunk_ranges(total: u64, chunk: u64) -> impl Iterator<Item = (u64, u64)> {
    (0..total.div_ceil(chunk)).map(move |index| {
        let start = index * chunk;
        (start, (start + chunk).min(total))
    })
}

async fn read_slice(blob: &web_sys::Blob, start: u64, end: u64) -> Result<Vec<u8>, JsValue> {
    let slice = blob
        .slice_with_f64_and_f64(to_f64(start), to_f64(end))
        .map_err(|error| crate::js_stage("slice file", &error))?;
    let buffer = JsFuture::from(slice.array_buffer())
        .await
        .map_err(|error| crate::js_stage("read file", &error))?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

fn blob_size(blob: &web_sys::Blob) -> u64 {
    // A JS number: integral and non-negative for any real file.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Blob.size is a non-negative integer well under 2^53"
    )]
    let size = blob.size() as u64;
    size
}

fn to_f64(value: u64) -> f64 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "file offsets stay far below 2^53, where f64 is exact"
    )]
    let value = value as f64;
    value
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::chunk_ranges;

    #[test]
    fn ranges_cover_the_file_exactly() {
        assert_eq!(chunk_ranges(0, 4).count(), 0);
        assert_eq!(chunk_ranges(4, 4).collect::<Vec<_>>(), [(0, 4)]);
        assert_eq!(
            chunk_ranges(10, 4).collect::<Vec<_>>(),
            [(0, 4), (4, 8), (8, 10)]
        );
    }
}
