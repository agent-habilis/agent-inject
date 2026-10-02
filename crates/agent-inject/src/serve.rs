//! Stand up one inject session: bind, mint the ticket, register the two
//! protocols on a Router.

use std::path::PathBuf;
use std::sync::Arc;

use agent_inject_proto::lookup::LookupOpts;
use agent_inject_proto::{Accept, InjectTicket, SECRET_LEN, UPLOAD_ALPN, WEBRTC_SIGNAL_ALPN};
use anyhow::{Context, Result, bail};
use fofoca::iroh::protocol::Router;
use fofoca::iroh::{EndpointAddr, SecretKey};
use fofoca_iroh_webrtc_transport::{IceConfig, WebRtcHandle, WebRtcTransport};
use tokio::sync::Notify;
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
    pub accept: Accept,
    /// Tests pass `IceConfig::host_only()`; the default asks public STUN.
    pub ice: IceConfig,
}

/// A running session. Dropping it without [`Session::shutdown`] leaves the
/// Router to be torn down with the runtime.
#[derive(Debug)]
pub struct Session {
    pub ticket: InjectTicket,
    /// The directory the files are written into, resolved.
    pub dir: PathBuf,
    /// Every saved file, as an absolute path, in the order it finished.
    pub saved: UnboundedReceiver<PathBuf>,
    done: Arc<Notify>,
    router: Router,
}

impl Session {
    /// Resolves when the sender says it has nothing more to send. Every file
    /// it sent before that is already in `saved`.
    ///
    /// The future owns its handle, so it can be awaited while `saved` is
    /// borrowed mutably, as in a `select!`.
    pub fn finished(&self) -> impl Future<Output = ()> + Send + use<> {
        let done = Arc::clone(&self.done);
        async move { done.notified().await }
    }

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
    let addr = if opts.lookups.is_loopback() {
        endpoint.addr()
    } else {
        wait_online(&endpoint).await;
        let Some(relay) = endpoint.addr().relay_urls().next().cloned() else {
            bail!("could not reach a relay; a browser has no other way to find this session");
        };
        // A browser has no UDP socket, so IP addresses in the ticket are
        // useless to it. Leaving them out keeps the URL (and so the QR code)
        // short, and keeps this machine's public IP out of a shared link.
        EndpointAddr::new(endpoint.id()).with_relay_url(relay)
    };

    let secret: [u8; SECRET_LEN] = rand::random();
    let ticket = InjectTicket {
        addr,
        secret,
        lookups: opts.lookups,
        accept: opts.accept,
    };
    let dir = opts
        .dir
        .canonicalize()
        .with_context(|| format!("resolve {}", opts.dir.display()))?;
    let ctx = Arc::new(ReceiveCtx::new(dir.clone(), secret, None, opts.accept));
    let (tx, saved) = unbounded_channel();
    let done = Arc::new(Notify::new());
    let router = Router::builder(endpoint.clone())
        .accept(UPLOAD_ALPN, UploadHandler::new(ctx, tx, Arc::clone(&done)))
        .accept(
            WEBRTC_SIGNAL_ALPN,
            SignalHandler::new(endpoint.id(), webrtc, opts.ice),
        )
        .spawn();
    Ok(Session {
        ticket,
        dir,
        saved,
        done,
        router,
    })
}
