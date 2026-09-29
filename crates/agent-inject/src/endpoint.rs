//! Build the receiver's iroh endpoint. Trimmed from agent-share's lookup
//! layer: inject only needs the relay (for the browser to reach us and to
//! carry the WebRTC signalling), so no mDNS or DHT lookup is wired.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use agent_inject_proto::lookup::{LookupOpts, RelayChoice};
use anyhow::{Context, Result, bail};
use fofoca::iroh::endpoint::{PortmapperConfig, presets};
use fofoca::iroh::{Endpoint, RelayMode, SecretKey};
use fofoca_iroh_webrtc_transport::WebRtcHandle;

/// Build an endpoint pinned to `key`.
///
/// A loopback session makes zero external network calls: bound to
/// 127.0.0.1, no relay, no portmapper. Otherwise the endpoint homes on the
/// relay ladder the ticket names.
///
/// With `webrtc`, the WebRTC transport is added beside iroh's own: additive,
/// so a native peer still prefers a direct IP path, and the path selector
/// keeps the relay a rendezvous rather than a data path.
pub(crate) async fn build_endpoint(
    lookups: &LookupOpts,
    key: SecretKey,
    webrtc: Option<&WebRtcHandle>,
) -> Result<Endpoint> {
    if lookups.mdns || lookups.dht {
        bail!("agent-inject wires no mDNS or DHT lookup");
    }
    let mut builder = if lookups.is_loopback() {
        Endpoint::builder(presets::Minimal)
            .bind_addr(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .context("failed to set bind address")?
            .relay_mode(RelayMode::Disabled)
            .portmapper_config(PortmapperConfig::Disabled)
    } else {
        Endpoint::builder(presets::Minimal).relay_mode(relay_mode(&lookups.relay))
    }
    .secret_key(key);
    if let Some(handle) = webrtc {
        builder = builder
            .add_custom_transport(handle.transport())
            .path_selector(handle.path_selector());
    }
    builder.bind().await.context("failed to bind endpoint")
}

/// Map a [`RelayChoice`] to the iroh [`RelayMode`].
fn relay_mode(choice: &RelayChoice) -> RelayMode {
    match choice {
        RelayChoice::Disabled => RelayMode::Disabled,
        RelayChoice::Custom(ladder) => RelayMode::custom(ladder.iter().cloned()),
        RelayChoice::Pinned => RelayMode::custom(pinned_ladder()),
    }
}

/// The `Pinned` ladder: the agent-habilis relay first, n0's as fallback.
/// Sourced from fofoca, so the CLI and the browser resolve "pinned" to the
/// same rungs; two copies that drifted would put the peers on different
/// relays with nothing to say why they never met.
pub(crate) fn pinned_ladder() -> Vec<fofoca::iroh::RelayUrl> {
    fofoca::net::relay_ladder(&fofoca::protocol::RelayChoice::Pinned)
}

/// Best-effort wait (≤5s) for the endpoint to reach its home relay, so the
/// printed ticket carries a relay URL the browser can dial.
pub(crate) async fn wait_online(endpoint: &Endpoint) {
    let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.online()).await;
}

#[cfg(test)]
mod tests {
    use agent_inject_proto::lookup::{LookupOpts, RelayChoice};
    use fofoca::iroh::SecretKey;

    use super::{build_endpoint, pinned_ladder, relay_mode};

    #[tokio::test]
    async fn loopback_binds_with_no_relay() {
        let key = SecretKey::from_bytes(&[4u8; 32]);
        let endpoint = build_endpoint(&LookupOpts::loopback(), key, None)
            .await
            .expect("loopback endpoint must bind");
        assert!(endpoint.addr().relay_urls().next().is_none());
        endpoint.close().await;
    }

    #[tokio::test]
    async fn mdns_and_dht_are_refused() {
        let key = SecretKey::from_bytes(&[4u8; 32]);
        assert!(
            build_endpoint(&LookupOpts::public_preset(), key, None)
                .await
                .is_err()
        );
    }

    /// Safari rejects the certificate of a host that ends with a dot, so a
    /// ticket naming such a relay never connects from an iPhone.
    #[test]
    fn no_pinned_relay_host_ends_with_a_dot() {
        for url in pinned_ladder() {
            let host = url.host_str().unwrap_or_default();
            assert!(!host.ends_with('.'), "{url}");
        }
    }

    #[test]
    fn pinned_offers_the_agent_habilis_relay() {
        let ladder = pinned_ladder();
        assert_eq!(
            ladder.first().and_then(|url| url.host_str()),
            Some("relay.agent-habilis.com")
        );
        let mut offered = relay_mode(&RelayChoice::Pinned)
            .relay_map()
            .urls::<Vec<_>>();
        let mut expected = ladder;
        offered.sort();
        expected.sort();
        assert_eq!(offered, expected);
    }
}
