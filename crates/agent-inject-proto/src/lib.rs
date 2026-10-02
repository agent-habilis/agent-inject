//! The `agent-inject` wire format — everything the CLI receiver and the
//! browser sender must agree on, and nothing else. Transport-free: no
//! endpoint, no tokio, no filesystem. Callers own the streams.
//!
//! - [`token`]: the Base58Check framing every agent-habilis token shares.
//! - [`ticket`]: the inject ticket — a bearer secret plus how to reach the
//!   receiver.
//! - [`framing`]: the ALPNs and the byte layout of one upload.
//! - [`lookup`]: the relay config baked into a ticket.
//! - [`peer_addr`]: the `EndpointAddr` JSON codec the ticket embeds.
//!
//! Everything here is wire format. The golden tests pin it, so a change that
//! shifts a byte fails loudly; when the change is intended, update the test in
//! the same commit.

pub use fofoca_protocol::ct_eq;

pub use self::framing::{
    CLOSE_UNAUTHORIZED, RequestHeader, Response, SECRET_LEN, Status, TRANSPORT, UPLOAD_ALPN,
    UPLOAD_ID_LEN, WEBRTC_SIGNAL_ALPN,
};
pub use self::ticket::{Accept, InjectTicket};

pub mod framing;
pub mod lookup;
pub mod peer_addr;
pub mod ticket;
pub mod token;
