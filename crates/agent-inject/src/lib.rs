//! `agent-inject` — receive photos and files from a phone into a directory,
//! peer to peer over iroh QUIC tunnelled through WebRTC.

use anyhow::Result;

pub(crate) mod util;

/// Parse argv and run the CLI end-to-end.
///
/// # Errors
/// Propagates any error from the session.
#[expect(clippy::unused_async, reason = "the session is async from M3 on")]
pub async fn run_cli() -> Result<()> {
    Ok(())
}
