//! Stand up one inject session: bind, mint the ticket, register the two
//! protocols on a Router.

use std::path::PathBuf;
use std::sync::Arc;

use agent_inject_proto::lookup::LookupOpts;
use agent_inject_proto::{InjectTicket, SECRET_LEN, UPLOAD_ALPN, WEBRTC_SIGNAL_ALPN};
use anyhow::{Context, Result, bail};
use fofoca::iroh::SecretKey;
use fofoca::iroh::protocol::Router;
use fofoca_iroh_webrtc_transport::{IceConfig, WebRtcHandle, WebRtcTransport};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::endpoint::{build_endpoint, wait_online};
use crate::handlers::{SignalHandler, UploadHandler};
use crate::receive::ReceiveCtx;

/// What a session is set up with.
#[derive(Debug)]
pub struct ServeOpts {
    /// Existing directory the files are written into.
    pub dir: PathBuf,
    pub lookups: LookupOpts,
    /// Tests pass `IceConfig::host_only()`; the default asks public STUN.
    pub ice: IceConfig,
}

/// A running session. Dropping it without [`Session::shutdown`] leaves the
/// Router to be torn down with the runtime.
#[derive(Debug)]
pub struct Session {
    pub ticket: InjectTicket,
    /// Every saved file, as an absolute path, in the order it finished.
    pub saved: UnboundedReceiver<PathBuf>,
    router: Router,
}

impl Session {
    /// Stop accepting, then close the endpoint. In-flight uploads are dropped
    /// and their `.part` files removed.
    pub async fn shutdown(self) {
        let _ = self.router.shutdown().await;
    }
}

/// Bind the endpoint, mint the ticket, and start serving.
///
/// # Errors
/// The endpoint cannot bind, or a public session has no relay URL for the
/// browser to reach it on.
pub async fn serve_with(opts: ServeOpts) -> Result<Session> {
    // The transport advertises `custom_addr(local_id)`, so it has to know the
    // endpoint's identity before the endpoint is built with it. Mint the key
    // first and pin it, or every WebRTC dial goes to an address nobody
    // listens on.
    let key = SecretKey::from_bytes(&rand::random());
    let webrtc = WebRtcHandle::new(WebRtcTransport::new(key.public()));
    let endpoint = build_endpoint(&opts.lookups, key, Some(&webrtc)).await?;
    debug_assert_eq!(endpoint.id(), webrtc.transport().local_id());
    if !opts.lookups.is_loopback() {
        wait_online(&endpoint).await;
        if endpoint.addr().relay_urls().next().is_none() {
            bail!("could not reach a relay; a browser has no other way to find this session");
        }
    }

    let secret: [u8; SECRET_LEN] = rand::random();
    let ticket = InjectTicket {
        addr: endpoint.addr(),
        secret,
        lookups: opts.lookups,
    };
    let dir = opts
        .dir
        .canonicalize()
        .with_context(|| format!("resolve {}", opts.dir.display()))?;
    let ctx = Arc::new(ReceiveCtx::new(dir, secret, None));
    let (tx, saved) = unbounded_channel();
    let router = Router::builder(endpoint.clone())
        .accept(UPLOAD_ALPN, UploadHandler::new(ctx, tx))
        .accept(
            WEBRTC_SIGNAL_ALPN,
            SignalHandler::new(endpoint.id(), webrtc, opts.ice),
        )
        .spawn();
    Ok(Session {
        ticket,
        saved,
        router,
    })
}
