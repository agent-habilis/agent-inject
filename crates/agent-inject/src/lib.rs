//! `agent-inject` — receive photos and files from a phone into a directory,
//! peer to peer over iroh QUIC tunnelled through WebRTC.

use anyhow::Result;
use clap::Parser;

pub(crate) mod cli;
pub(crate) mod endpoint;
pub(crate) mod handlers;
pub(crate) mod plug;
pub(crate) mod qr;
pub(crate) mod receive;
pub(crate) mod serve;
pub(crate) mod session_dir;
pub(crate) mod util;
pub(crate) mod web;
pub(crate) mod webrtc;

/// Parse argv and run the CLI end-to-end.
///
/// # Errors
/// Propagates any error from the session.
pub async fn run_cli() -> Result<()> {
    cli::run(cli::Cli::parse()).await
}

/// Internals reached by the integration tests. Not part of the CLI's
/// contract.
#[doc(hidden)]
pub mod test_support {
    use agent_inject_proto::lookup::LookupOpts;
    use agent_inject_proto::{RequestHeader, Response};
    use anyhow::{Context, Result};
    use fofoca::iroh::endpoint::Connection;
    use fofoca::iroh::{Endpoint, SecretKey};

    pub use crate::serve::{ServeOpts, Session, serve_with};
    pub use crate::webrtc::{dial_webrtc, ensure_webrtc_selected, webrtc_only_addr};

    /// A sender endpoint on 127.0.0.1 with no relay and no WebRTC.
    ///
    /// # Errors
    /// The endpoint cannot bind.
    pub async fn loopback_sender() -> Result<Endpoint> {
        let key = SecretKey::from_bytes(&rand::random());
        crate::endpoint::build_endpoint(&LookupOpts::loopback(), key, None).await
    }

    /// Send the done signal on `conn` the way the browser does, and read the
    /// answer.
    ///
    /// # Errors
    /// The stream fails before a response arrives.
    pub async fn finish(conn: &Connection, secret: &[u8; 32]) -> Result<Response> {
        let (mut send, mut recv) = conn.open_bi().await.context("open done stream")?;
        send.write_all(&agent_inject_proto::framing::encode_done(secret))
            .await
            .context("send done")?;
        let _ = send.finish();
        let raw = recv.read_to_end(4096).await.context("read response")?;
        Response::decode(&raw)
    }

    /// Send one upload on `conn` the way the browser does, and read the
    /// answer.
    ///
    /// # Errors
    /// The stream fails before a response arrives.
    pub async fn upload(
        conn: &Connection,
        header: &RequestHeader,
        body: &[u8],
    ) -> Result<Response> {
        let (mut send, mut recv) = conn.open_bi().await.context("open upload stream")?;
        send.write_all(&header.encode()?)
            .await
            .context("send header")?;
        // A refused upload stops our stream; the response still says why.
        let _ = send.write_all(body).await;
        let _ = send.finish();
        let raw = recv.read_to_end(4096).await.context("read response")?;
        Response::decode(&raw)
    }
}
