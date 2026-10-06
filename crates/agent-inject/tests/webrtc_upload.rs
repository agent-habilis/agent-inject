//! An upload over a real WebRTC data channel: the lane a browser uses.
//!
//! The receiver is the real session (Router, `SignalHandler`,
//! `UploadHandler`). The sender is built the way the browser client is: two
//! endpoints on one key. The signal endpoint negotiates JSEP; the upload
//! endpoint has no IP and no relay transport, only WebRTC. So the upload has
//! no path but the data channel, and the test also asserts that path is the
//! selected one.

use agent_inject::test_support::{
    ServeOpts, dial_webrtc, ensure_webrtc_selected, serve_with, upload, webrtc_only_addr,
};
use agent_inject_proto::lookup::LookupOpts;
use agent_inject_proto::{Accept, RequestHeader, Status, UPLOAD_ALPN};
use habilis_network::iroh::endpoint::presets;
use habilis_network::iroh::{Endpoint, RelayMode, SecretKey};
use habilis_network_iroh_webrtc_transport::{IceConfig, WebRtcHandle, WebRtcTransport};
use sha2::{Digest, Sha256};

/// Host candidates only: the default config asks public STUN, which would
/// make this test depend on the network.
fn ice() -> IceConfig {
    IceConfig::host_only()
}

async fn browser_like_sender() -> (Endpoint, Endpoint, WebRtcHandle) {
    let key = SecretKey::from_bytes(&rand::random());
    let handle = WebRtcHandle::new(WebRtcTransport::new(key.public()));
    let signal = Endpoint::builder(presets::Minimal)
        .secret_key(key.clone())
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .expect("bind signal endpoint");
    let data = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .clear_ip_transports()
        .clear_relay_transports()
        .add_custom_transport(handle.transport())
        .bind()
        .await
        .expect("bind upload endpoint");
    (signal, data, handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upload_rides_the_webrtc_data_channel() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = serve_with(ServeOpts {
        dir: dir.path().to_owned(),
        lookups: LookupOpts::loopback(),
        accept: Accept::Any,
        ice: ice(),
    })
    .await
    .expect("serve");

    let (signal, data, handle) = browser_like_sender().await;
    let receiver = session.ticket.addr.clone();
    let addr = Box::pin(dial_webrtc(&signal, receiver.clone(), &handle, &ice()))
        .await
        .expect("negotiate WebRTC");
    assert_eq!(addr, webrtc_only_addr(receiver.id));

    let conn = data
        .connect(addr, UPLOAD_ALPN)
        .await
        .expect("dial upload over WebRTC");
    ensure_webrtc_selected(&conn, "sender")
        .await
        .expect("the upload must ride the data channel");

    let body: Vec<u8> = (0..8u32 * 1024 * 1024)
        .map(|index| (index % 253) as u8)
        .collect();
    let header = RequestHeader {
        secret: session.ticket.secret,
        upload_id: [1u8; 16],
        name: "photo.jpg".to_owned(),
        size: body.len() as u64,
    };
    let response = upload(&conn, &header, &body).await.expect("upload");
    assert_eq!(response.status, Status::Ok);
    assert_eq!(response.message, "photo.jpg");

    let saved = session.saved.recv().await.unwrap();
    assert_eq!(
        Sha256::digest(std::fs::read(saved).unwrap()),
        Sha256::digest(&body)
    );
    session.shutdown().await;
}
