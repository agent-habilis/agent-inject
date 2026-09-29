//! The ALPNs and the byte layout of one upload.
//!
//! One upload is one QUIC bi-stream, opened by the browser:
//!
//! ```text
//! request:  secret(32) ‖ upload_id(16) ‖ name_len(u16 LE) ‖ name ‖ size(u64 LE) ‖ body(size) ‖ FIN
//! response: status(u8) ‖ len(u16 LE) ‖ message(UTF-8)
//! ```
//!
//! On `Ok` the message is the basename the file was saved under, which can
//! differ from the requested name after sanitizing or a collision suffix. On
//! any other status it is the reason.
//!
//! `upload_id` is random per file on the client. A retry after a lost ack
//! reuses it, so the receiver can answer with the name it already saved
//! instead of writing a second copy.
//!
//! Sans-io: the receiver reads [`REQUEST_PREFIX_LEN`] bytes, asks
//! [`remaining_header_len`] how many more make up the header, reads those,
//! then calls [`RequestHeader::decode`] on the whole header.

use anyhow::{Context, Result, bail};
use fofoca_protocol::TransportPolicy;

/// The upload protocol.
pub const UPLOAD_ALPN: &[u8] = b"agent-inject/upload/1";

/// The SDP offer/answer exchange that sets up the WebRTC data channel the
/// upload connection then runs over.
pub const WEBRTC_SIGNAL_ALPN: &[u8] = b"agent-inject/webrtc-signal/1";

/// What may carry upload bytes, in fofoca's terms. A WebRTC data channel
/// first; when ICE cannot open one (a phone behind carrier NAT), the relay.
/// No UDP: the sender is a browser, which has none. How the two ends find
/// each other is the relay lookup the ticket names.
pub const TRANSPORT: TransportPolicy = TransportPolicy {
    udp: false,
    webrtc: true,
    relay_transport: true,
};

/// Bearer secret carried by the ticket and presented on every upload.
pub const SECRET_LEN: usize = 32;

/// Per-file id chosen by the client.
pub const UPLOAD_ID_LEN: usize = 16;

/// Ceiling on the requested file name. Far above any real name; it only
/// bounds what a forged header can make the receiver allocate.
pub const MAX_NAME_BYTES: usize = 1024;

/// Ceiling on a response message, for the same reason.
pub const MAX_MESSAGE_BYTES: usize = 1024;

/// `secret ‖ upload_id ‖ name_len`: the fixed part in front of the name.
pub const REQUEST_PREFIX_LEN: usize = SECRET_LEN + UPLOAD_ID_LEN + 2;

/// Connection close code for a request that presents the wrong secret.
pub const CLOSE_UNAUTHORIZED: u32 = 1;

/// The header of one upload, everything in front of the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHeader {
    pub secret: [u8; SECRET_LEN],
    pub upload_id: [u8; UPLOAD_ID_LEN],
    pub name: String,
    pub size: u64,
}

impl RequestHeader {
    /// Encode the header. The body follows it on the stream.
    ///
    /// # Errors
    /// The name is over [`MAX_NAME_BYTES`].
    pub fn encode(&self) -> Result<Vec<u8>> {
        let name_len = checked_name_len(self.name.len())?;
        let mut buf = Vec::with_capacity(REQUEST_PREFIX_LEN + self.name.len() + 8);
        buf.extend_from_slice(&self.secret);
        buf.extend_from_slice(&self.upload_id);
        buf.extend_from_slice(&name_len.to_le_bytes());
        buf.extend_from_slice(self.name.as_bytes());
        buf.extend_from_slice(&self.size.to_le_bytes());
        Ok(buf)
    }

    /// Decode a whole header: the prefix plus [`remaining_header_len`] bytes.
    ///
    /// # Errors
    /// Wrong length, a name over [`MAX_NAME_BYTES`], or a name that is not
    /// UTF-8.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let prefix = bytes
            .get(..REQUEST_PREFIX_LEN)
            .context("truncated request prefix")?;
        let name_len = remaining_header_len(prefix)? - 8;
        if bytes.len() != REQUEST_PREFIX_LEN + name_len + 8 {
            bail!("request header length mismatch");
        }
        let mut secret = [0u8; SECRET_LEN];
        secret.copy_from_slice(&bytes[..SECRET_LEN]);
        let mut upload_id = [0u8; UPLOAD_ID_LEN];
        upload_id.copy_from_slice(&bytes[SECRET_LEN..SECRET_LEN + UPLOAD_ID_LEN]);
        let name_end = REQUEST_PREFIX_LEN + name_len;
        let name = std::str::from_utf8(&bytes[REQUEST_PREFIX_LEN..name_end])
            .context("file name is not UTF-8")?
            .to_owned();
        let mut size = [0u8; 8];
        size.copy_from_slice(&bytes[name_end..]);
        Ok(Self {
            secret,
            upload_id,
            name,
            size: u64::from_le_bytes(size),
        })
    }
}

/// How many header bytes follow the prefix: the name plus the size field.
///
/// # Errors
/// The prefix is short, or declares a name over [`MAX_NAME_BYTES`].
pub fn remaining_header_len(prefix: &[u8]) -> Result<usize> {
    let len_bytes = prefix
        .get(SECRET_LEN + UPLOAD_ID_LEN..REQUEST_PREFIX_LEN)
        .context("truncated request prefix")?;
    let name_len = usize::from(u16::from_le_bytes([len_bytes[0], len_bytes[1]]));
    if name_len > MAX_NAME_BYTES {
        bail!("file name too long: {name_len} bytes");
    }
    Ok(name_len + 8)
}

fn checked_name_len(len: usize) -> Result<u16> {
    if len > MAX_NAME_BYTES {
        bail!("file name too long: {len} bytes");
    }
    Ok(u16::try_from(len).expect("MAX_NAME_BYTES fits in u16"))
}

/// The outcome of one upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Unauthorized,
    BadName,
    Truncated,
    Io,
    TooLarge,
}

impl Status {
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Status::Ok => 0,
            Status::Unauthorized => 1,
            Status::BadName => 2,
            Status::Truncated => 3,
            Status::Io => 4,
            Status::TooLarge => 5,
        }
    }

    /// # Errors
    /// An unknown status byte.
    pub fn from_byte(byte: u8) -> Result<Self> {
        Ok(match byte {
            0 => Status::Ok,
            1 => Status::Unauthorized,
            2 => Status::BadName,
            3 => Status::Truncated,
            4 => Status::Io,
            5 => Status::TooLarge,
            other => bail!("unknown upload status: {other}"),
        })
    }

    /// Stable lowercase name, used as the error code in the web client.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Unauthorized => "unauthorized",
            Status::BadName => "bad_name",
            Status::Truncated => "truncated",
            Status::Io => "io",
            Status::TooLarge => "too_large",
        }
    }
}

/// The receiver's answer to one upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: Status,
    pub message: String,
}

impl Response {
    /// Encode the response. A message over [`MAX_MESSAGE_BYTES`] is cut on a
    /// char boundary rather than refused: the status is what matters, and the
    /// receiver must always be able to answer.
    ///
    /// # Panics
    /// Never: the cut keeps the message under [`MAX_MESSAGE_BYTES`], which
    /// fits in a `u16`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut end = self.message.len().min(MAX_MESSAGE_BYTES);
        while !self.message.is_char_boundary(end) {
            end -= 1;
        }
        let message = &self.message[..end];
        let len = u16::try_from(message.len()).expect("MAX_MESSAGE_BYTES fits in u16");
        let mut buf = Vec::with_capacity(3 + message.len());
        buf.push(self.status.to_byte());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(message.as_bytes());
        buf
    }

    /// # Errors
    /// Truncated bytes, an unknown status, or a message that is not UTF-8.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let status = Status::from_byte(*bytes.first().context("empty response")?)?;
        let len_bytes = bytes.get(1..3).context("truncated response length")?;
        let len = usize::from(u16::from_le_bytes([len_bytes[0], len_bytes[1]]));
        let raw = bytes
            .get(3..3 + len)
            .context("truncated response message")?;
        if bytes.len() != 3 + len {
            bail!("trailing bytes after response");
        }
        let message = std::str::from_utf8(raw)
            .context("response message is not UTF-8")?
            .to_owned();
        Ok(Self { status, message })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_MESSAGE_BYTES, MAX_NAME_BYTES, REQUEST_PREFIX_LEN, RequestHeader, Response, Status,
        TRANSPORT, UPLOAD_ALPN, WEBRTC_SIGNAL_ALPN, remaining_header_len,
    };

    fn header(name: &str) -> RequestHeader {
        RequestHeader {
            secret: [7u8; 32],
            upload_id: [9u8; 16],
            name: name.to_owned(),
            size: 0x0102_0304_0506_0708,
        }
    }

    #[test]
    fn payload_rides_webrtc_then_the_relay() {
        assert!(TRANSPORT.webrtc);
        assert!(TRANSPORT.relay_transport);
        assert!(!TRANSPORT.udp, "a browser has no UDP");
    }

    #[test]
    fn wire_constants_are_pinned() {
        assert_eq!(UPLOAD_ALPN, b"agent-inject/upload/1");
        assert_eq!(WEBRTC_SIGNAL_ALPN, b"agent-inject/webrtc-signal/1");
        assert_eq!(REQUEST_PREFIX_LEN, 50);
    }

    #[test]
    fn header_bytes_are_pinned() {
        let bytes = header("a.jpg").encode().unwrap();
        let mut expected = vec![7u8; 32];
        expected.extend([9u8; 16]);
        expected.extend([5, 0]);
        expected.extend(b"a.jpg");
        expected.extend([8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn header_round_trips_through_the_prefix_protocol() {
        let original = header("IMG_0001 café.heic");
        let bytes = original.encode().unwrap();
        let rest = remaining_header_len(&bytes[..REQUEST_PREFIX_LEN]).unwrap();
        assert_eq!(REQUEST_PREFIX_LEN + rest, bytes.len());
        assert_eq!(RequestHeader::decode(&bytes).unwrap(), original);
    }

    #[test]
    fn empty_name_and_zero_size_round_trip() {
        let original = RequestHeader {
            size: 0,
            ..header("")
        };
        let bytes = original.encode().unwrap();
        assert_eq!(RequestHeader::decode(&bytes).unwrap(), original);
    }

    #[test]
    fn name_over_the_cap_is_rejected_both_ways() {
        let long = "x".repeat(MAX_NAME_BYTES + 1);
        assert!(header(&long).encode().is_err());

        let mut prefix = vec![0u8; REQUEST_PREFIX_LEN];
        let len = u16::try_from(MAX_NAME_BYTES + 1).unwrap().to_le_bytes();
        prefix[48..50].copy_from_slice(&len);
        assert!(remaining_header_len(&prefix).is_err());
    }

    #[test]
    fn non_utf8_name_and_bad_length_are_rejected() {
        let mut bytes = header("ab").encode().unwrap();
        bytes[REQUEST_PREFIX_LEN] = 0xff;
        assert!(RequestHeader::decode(&bytes).is_err());

        let mut short = header("ab").encode().unwrap();
        short.pop();
        assert!(RequestHeader::decode(&short).is_err());
    }

    #[test]
    fn status_bytes_are_pinned() {
        for (status, byte) in [
            (Status::Ok, 0u8),
            (Status::Unauthorized, 1),
            (Status::BadName, 2),
            (Status::Truncated, 3),
            (Status::Io, 4),
            (Status::TooLarge, 5),
        ] {
            assert_eq!(status.to_byte(), byte);
            assert_eq!(Status::from_byte(byte).unwrap(), status);
        }
        assert!(Status::from_byte(6).is_err());
    }

    #[test]
    fn response_round_trips_and_pins_its_bytes() {
        let response = Response {
            status: Status::Ok,
            message: "a-2.jpg".to_owned(),
        };
        let bytes = response.encode();
        assert_eq!(bytes[..3], [0, 7, 0]);
        assert_eq!(Response::decode(&bytes).unwrap(), response);
    }

    #[test]
    fn long_response_message_is_cut_on_a_char_boundary() {
        let response = Response {
            status: Status::Io,
            message: "é".repeat(MAX_MESSAGE_BYTES),
        };
        let decoded = Response::decode(&response.encode()).unwrap();
        assert!(decoded.message.len() <= MAX_MESSAGE_BYTES);
        assert!(decoded.message.chars().all(|ch| ch == 'é'));
    }

    #[test]
    fn response_with_trailing_bytes_is_rejected() {
        let mut bytes = Response {
            status: Status::Ok,
            message: "a".to_owned(),
        }
        .encode();
        bytes.push(0);
        assert!(Response::decode(&bytes).is_err());
    }
}
