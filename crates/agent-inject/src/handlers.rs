//! The session's two protocols, as `ProtocolHandler`s on one Router.
//!
//! The Router owns the accept loop: iroh allows exactly one per endpoint, and
//! `Router::spawn` overrides any ALPNs set when the endpoint was built.

use std::path::PathBuf;
use std::sync::Arc;

use agent_inject_proto::{CLOSE_UNAUTHORIZED, Status};
use fofoca::iroh::EndpointId;
use fofoca::iroh::endpoint::Connection;
use fofoca::iroh::protocol::{AcceptError, ProtocolHandler};
use fofoca_iroh_webrtc_transport::{IceConfig, WebRtcHandle};
use tokio::sync::mpsc::UnboundedSender;

use crate::receive::{Outcome, ReceiveCtx, receive_core};

/// Serves `UPLOAD_ALPN`: one connection per browser tab, one upload per
/// bi-stream. Each saved path goes to `saved`, in the order the files finish.
#[derive(Debug, Clone)]
pub(crate) struct UploadHandler {
    ctx: Arc<ReceiveCtx>,
    saved: UnboundedSender<PathBuf>,
}

impl UploadHandler {
    pub(crate) fn new(ctx: Arc<ReceiveCtx>, saved: UnboundedSender<PathBuf>) -> Self {
        Self { ctx, saved }
    }
}

impl ProtocolHandler for UploadHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // Ends when the connection does: the tab closed, or a wrong secret
        // closed it from inside a stream task.
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let ctx = Arc::clone(&self.ctx);
            let saved = self.saved.clone();
            let conn = conn.clone();
            tokio::spawn(async move {
                // One failed upload must never end the connection or the
                // session, so every error stops here.
                match receive_core(&ctx, &mut recv, &mut send).await {
                    Ok(Outcome::Saved(path)) => {
                        let _ = saved.send(path);
                    }
                    Ok(Outcome::AlreadySaved(_)) => {}
                    Ok(Outcome::Refused(Status::Unauthorized)) => {
                        let _ = send.stopped().await;
                        conn.close(CLOSE_UNAUTHORIZED.into(), b"unauthorized");
                    }
                    Ok(Outcome::Refused(status)) => {
                        // Tell the sender to stop writing a body we will not
                        // read; it then reads the response for the reason.
                        let _ = recv.stop(0u32.into());
                        tracing::debug!(status = status.label(), "upload refused");
                    }
                    Err(error) => tracing::debug!(%error, "upload stream ended"),
                }
            });
        }
        Ok(())
    }
}

/// Serves `WEBRTC_SIGNAL_ALPN`: one JSEP exchange, then done.
#[derive(Debug, Clone)]
pub(crate) struct SignalHandler {
    local: EndpointId,
    webrtc: WebRtcHandle,
    ice: IceConfig,
}

impl SignalHandler {
    pub(crate) fn new(local: EndpointId, webrtc: WebRtcHandle, ice: IceConfig) -> Self {
        Self { local, webrtc, ice }
    }
}

impl ProtocolHandler for SignalHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(error) =
            crate::webrtc::serve_signal(&conn, self.local, &self.webrtc, &self.ice).await
        {
            tracing::debug!(%error, "webrtc signalling ended");
        }
        Ok(())
    }
}
