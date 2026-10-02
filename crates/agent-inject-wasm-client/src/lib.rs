//! The browser half of agent-inject: dial the receiver named by a ticket over
//! a WebRTC data channel and stream files to it.
//!
//! The connect path is agent-share's browser client, trimmed to one peer and
//! no mesh. TypeScript never touches the wire format; it calls these exports.

use std::sync::Arc;

use agent_inject_proto::lookup::RelayChoice;
use agent_inject_proto::{
    InjectTicket, SECRET_LEN, TRANSPORT, UPLOAD_ALPN, UPLOAD_ID_LEN, WEBRTC_SIGNAL_ALPN,
};
use fofoca::iroh::endpoint::{Connection, presets};
use fofoca::iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr};
use fofoca_iroh_webrtc_transport::{
    BrowserHubTransport, BrowserSession, IceServers, MAX_ENVELOPE_BYTES, SignalEnvelope,
    WebRtcHandle, browser_offer, custom_addr,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

mod upload;

/// How long to wait for the upload connection to select a path.
const PATH_SETTLE_MS: f64 = 3_000.0;

/// One tab's connection to one receiver.
#[wasm_bindgen]
pub struct InjectClient {
    connection: Connection,
    secret: [u8; SECRET_LEN],
    /// `webrtc` or `relay`: the path the upload connection rides.
    data_path: &'static str,
    // Held so the transport and both endpoints live as long as the client.
    endpoint: Endpoint,
    signal_endpoint: Endpoint,
    _hub: Arc<BrowserHubTransport>,
    _session: Option<BrowserSession>,
}

#[wasm_bindgen]
impl InjectClient {
    /// Validate a ticket without dialling, and return what the session
    /// accepts (`any` or `images`), so the page shows the right pickers
    /// before it connects.
    ///
    /// # Errors
    /// The ticket does not decode.
    #[wasm_bindgen(js_name = parseTicket)]
    pub fn parse_ticket(ticket: &str) -> Result<String, JsValue> {
        InjectTicket::decode(ticket)
            .map(|ticket| ticket.accept.label().to_owned())
            .map_err(|error| err("decode ticket", &error))
    }

    /// Dial the receiver named by `ticket`: over a WebRTC data channel, or
    /// over the relay when that fails (see [`TRANSPORT`]).
    ///
    /// The page's debugging overrides can only narrow that policy: `webrtc`
    /// and `relay` switch a path off, and `relay_urls` names the relays to
    /// home on instead of the ticket's ladder.
    ///
    /// # Errors
    /// The ticket or a relay URL does not parse, both paths are switched off,
    /// or no allowed path connects.
    pub async fn connect(
        ticket: String,
        webrtc: Option<bool>,
        relay: Option<bool>,
        relay_urls: Option<Vec<String>>,
    ) -> Result<InjectClient, JsValue> {
        console_error_panic_hook::set_once();
        let ticket = InjectTicket::decode(&ticket).map_err(|error| err("decode ticket", &error))?;
        let paths = Paths {
            webrtc: TRANSPORT.webrtc && webrtc.unwrap_or(true),
            relay: TRANSPORT.relay_transport && relay.unwrap_or(true),
        };
        if !paths.webrtc && !paths.relay {
            return Err(JsValue::from_str(
                "no transport left: allow webrtc, relay, or both",
            ));
        }
        let home = signal_relay_mode(&ticket.lookups.relay, &relay_urls.unwrap_or_default())?;
        connect_any(ticket, paths, home).await
    }

    /// `webrtc` or `relay`: the path the upload connection rides.
    #[wasm_bindgen(js_name = dataPath)]
    pub fn data_path(&self) -> String {
        self.data_path.to_owned()
    }

    /// Stream `blob` to the receiver under `name`. `upload_id` is 16 random
    /// bytes, reused on a retry so a lost ack cannot make a second copy.
    /// `on_progress(sent, total)` fires after each chunk the stream accepts.
    ///
    /// Resolves with the name the receiver saved the file under. Rejects with
    /// `"<code>: <reason>"`, where `code` is a stable status label such as
    /// `unauthorized` or `truncated`, or with a plain message when the stream
    /// failed.
    ///
    /// # Errors
    /// See above.
    pub async fn upload(
        &self,
        name: String,
        blob: web_sys::Blob,
        upload_id: Vec<u8>,
        on_progress: Option<js_sys::Function>,
    ) -> Result<String, JsValue> {
        let upload_id: [u8; UPLOAD_ID_LEN] = upload_id
            .try_into()
            .map_err(|_| JsValue::from_str("upload id must be 16 bytes"))?;
        upload::send(
            &self.connection,
            self.secret,
            upload_id,
            name,
            &blob,
            on_progress.as_ref(),
        )
        .await
    }

    /// Tell the receiver the sender is finished; the receiver then ends the
    /// session. Resolves with how many files it saved.
    ///
    /// # Errors
    /// The stream fails, or the receiver refuses.
    pub async fn finish(&self) -> Result<u32, JsValue> {
        upload::send_done(&self.connection, &self.secret).await
    }

    /// Resolves when the connection ends, with the reason. The page reconnects
    /// from here.
    pub async fn closed(&self) -> String {
        self.connection.closed().await.to_string()
    }

    /// Close the connection and both endpoints.
    pub async fn close(&self) {
        self.connection.close(0u32.into(), b"closed");
        self.endpoint.close().await;
        self.signal_endpoint.close().await;
    }
}

/// Bind the two endpoints, then connect on the first path [`TRANSPORT`]
/// allows that works: the data channel, then the relay.
///
/// **Two** endpoints on one key, and the split is what puts bytes on the data
/// channel at all. The JSEP exchange rides the relay, so after it the signal
/// endpoint's address book holds a warm relay path to the receiver; iroh only
/// fans a connect out while the remote has no selected path, so a second dial
/// on that endpoint would ride the relay. The upload endpoint has no relay and
/// (in a tab) no IP, so the data channel is its only path. That warm relay
/// path is exactly what the fallback wants, so the fallback dials on the
/// signal endpoint.
///
/// Only the signal endpoint may hold the relay: two same-key endpoints both
/// registering with one relay fight over the registration and ICE never
/// completes.
/// Which paths this connect may use.
#[derive(Debug, Clone, Copy)]
struct Paths {
    webrtc: bool,
    relay: bool,
}

/// The relays the signal endpoint homes on: the override when there is one,
/// else the ladder the ticket names.
fn signal_relay_mode(choice: &RelayChoice, overrides: &[String]) -> Result<RelayMode, JsValue> {
    if overrides.is_empty() {
        return Ok(relay_mode(choice));
    }
    let urls = overrides
        .iter()
        .map(|raw| {
            raw.parse::<fofoca::iroh::RelayUrl>()
                .map_err(|error| err(&format!("relay URL {raw}"), &error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RelayMode::custom(urls))
}

async fn connect_any(
    ticket: InjectTicket,
    paths: Paths,
    home: RelayMode,
) -> Result<InjectClient, JsValue> {
    ensure_reachable_addr(&ticket.addr)?;

    let key = SecretKey::generate();
    let local = key.public();
    let signal_bind = async {
        Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .relay_mode(home)
            .bind()
            .await
            .map_err(|error| err("bind signal endpoint", &error))
    };
    let hub = BrowserHubTransport::new(local);
    let handle = WebRtcHandle::new(Arc::clone(&hub));
    let upload_bind = async {
        Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .relay_mode(RelayMode::Disabled)
            .add_custom_transport(handle.transport())
            .path_selector(handle.path_selector())
            .bind()
            .await
            .map_err(|error| err("bind upload endpoint", &error))
    };
    let (signal_endpoint, endpoint) = futures::future::try_join(signal_bind, upload_bind).await?;

    let webrtc = if !paths.webrtc {
        Err(JsValue::from_str("WebRTC is switched off"))
    } else {
        dial_webrtc(&signal_endpoint, &endpoint, &hub, &ticket, local).await
    };
    let (connection, session, data_path) = match webrtc {
        Ok((connection, session)) => (connection, Some(session), "webrtc"),
        Err(webrtc_error) if paths.relay => {
            web_sys::console::warn_1(&JsValue::from_str(&format!(
                "[agent-inject] WebRTC failed, using the relay: {}",
                describe(&webrtc_error)
            )));
            match signal_endpoint
                .connect(ticket.addr.clone(), UPLOAD_ALPN)
                .await
            {
                Ok(connection) => (connection, None, "relay"),
                Err(error) => {
                    endpoint.close().await;
                    signal_endpoint.close().await;
                    return Err(JsValue::from_str(&format!(
                        "WebRTC failed ({}), and the relay failed too ({error})",
                        describe(&webrtc_error)
                    )));
                }
            }
        }
        Err(error) => {
            endpoint.close().await;
            signal_endpoint.close().await;
            return Err(error);
        }
    };
    Ok(InjectClient {
        connection,
        secret: ticket.secret,
        data_path,
        endpoint,
        signal_endpoint,
        _hub: hub,
        _session: session,
    })
}

/// Negotiate a data channel, then dial the upload ALPN over it alone.
async fn dial_webrtc(
    signal_endpoint: &Endpoint,
    endpoint: &Endpoint,
    hub: &BrowserHubTransport,
    ticket: &InjectTicket,
    local: EndpointId,
) -> Result<(Connection, BrowserSession), JsValue> {
    let receiver = ticket.addr.id;
    let session = negotiate(signal_endpoint, ticket.addr.clone(), local, hub).await?;
    let webrtc_only =
        EndpointAddr::from_parts(receiver, [TransportAddr::Custom(custom_addr(receiver))]);
    let connection = endpoint
        .connect(webrtc_only, UPLOAD_ALPN)
        .await
        .map_err(|error| err("dial the upload ALPN over WebRTC", &error))?;
    // The *selected* path, not "a WebRTC path exists": a connection can hold
    // a path it does not send on.
    let selected = settled_path_label(&connection).await;
    if selected.as_deref() != Some("webrtc") {
        let observed = path_labels(&connection);
        connection.close(0u32.into(), b"not webrtc");
        return Err(JsValue::from_str(&format!(
            "connected but selected {} rather than WebRTC (paths={observed:?})",
            selected.as_deref().unwrap_or("no path"),
        )));
    }
    Ok((connection, session))
}

/// A `JsValue` error as one line of prose.
fn describe(error: &JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

fn ensure_reachable_addr(addr: &EndpointAddr) -> Result<(), JsValue> {
    if addr.relay_urls().next().is_none() {
        return Err(JsValue::from_str(
            "ticket has no relay address — the receiver was not reachable when the link was made",
        ));
    }
    Ok(())
}

/// Swap one JSEP envelope each way over the signal ALPN, then attach into
/// `hub`.
async fn negotiate(
    endpoint: &Endpoint,
    receiver: EndpointAddr,
    local: EndpointId,
    hub: &BrowserHubTransport,
) -> Result<BrowserSession, JsValue> {
    let receiver_id = receiver.id;
    // The relay dial and the offer are independent, so the dial's latency
    // hides inside ICE gathering.
    let dial = async {
        let conn = endpoint
            .connect(receiver, WEBRTC_SIGNAL_ALPN)
            .await
            .map_err(|error| err("dial the signal ALPN", &error))?;
        let streams = conn
            .open_bi()
            .await
            .map_err(|error| err("open signal stream", &error))?;
        Ok((conn, streams))
    };
    // STUN only: TURN is refused by policy, and the iroh relay never carries
    // file data.
    let ice = IceServers::default();
    let build_offer = async {
        browser_offer(local, &ice)
            .await
            .map_err(|error| js_stage("build offer", &error))
    };
    let ((conn, (mut send, mut recv)), (pending, offer)) =
        futures::future::try_join(dial, build_offer).await?;
    let encoded = serde_json::to_vec(&offer).map_err(|error| err("encode offer", &error))?;
    send.write_all(&encoded)
        .await
        .map_err(|error| err("send offer", &error))?;
    send.finish().map_err(|error| err("finish", &error))?;

    let raw = recv
        .read_to_end(MAX_ENVELOPE_BYTES)
        .await
        .map_err(|error| err("read answer (is agent-inject still running?)", &error))?;
    let answer: SignalEnvelope =
        serde_json::from_slice(&raw).map_err(|error| err("parse answer", &error))?;
    if let SignalEnvelope::Error { reason, .. } = &answer {
        return Err(JsValue::from_str(&format!(
            "receiver refused WebRTC signalling: {reason}"
        )));
    }
    let session = pending
        .complete(hub, &answer)
        .await
        .map_err(|error| js_stage("complete WebRTC offer", &error))?;
    // The hub is keyed by the answer's claimed id and the dial uses the
    // ticket's. A mismatch would blackhole every transmit with no error.
    if session.remote != receiver_id {
        return Err(JsValue::from_str(&format!(
            "answer endpoint id {} does not match the ticket's {receiver_id}",
            session.remote
        )));
    }
    conn.close(0u32.into(), b"jsep done");
    Ok(session)
}

/// Wait for path selection, then label the path that won. `None` means
/// nothing was selected before the deadline.
async fn settled_path_label(conn: &Connection) -> Option<String> {
    let deadline = js_sys::Date::now() + PATH_SETTLE_MS;
    loop {
        let selected = {
            let paths = conn.paths();
            paths
                .iter()
                .find(|path| path.is_selected())
                .map(|path| path_label(path.remote_addr()))
        };
        if selected.is_some() {
            return selected;
        }
        if js_sys::Date::now() >= deadline {
            return None;
        }
        wait_ms(50).await;
    }
}

fn path_labels(conn: &Connection) -> Vec<String> {
    conn.paths()
        .iter()
        .map(|path| path_label(path.remote_addr()))
        .collect()
}

fn path_label(addr: &TransportAddr) -> String {
    match addr {
        TransportAddr::Relay(_) => "relay".to_owned(),
        TransportAddr::Ip(_) => "ip".to_owned(),
        TransportAddr::Custom(custom)
            if custom.id() == fofoca_iroh_webrtc_transport::WEBRTC_TRANSPORT_ID =>
        {
            "webrtc".to_owned()
        }
        other => format!("{other:?}"),
    }
}

async fn wait_ms(millis: i32) {
    // `setTimeout` off the global rather than the `Window`, so this also
    // resolves in the node test runner, which has no `Window`.
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        use wasm_bindgen::JsCast as _;
        let global = js_sys::global();
        if let Ok(set_timeout) = js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout")) {
            let set_timeout: js_sys::Function = set_timeout.unchecked_into();
            let _ = set_timeout.call2(&global, &resolve, &JsValue::from(millis));
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// The relay ladder the ticket names. `Pinned` resolves through fofoca, the
/// same list the CLI uses, so both ends home on the same rungs.
fn relay_mode(choice: &RelayChoice) -> RelayMode {
    match choice {
        RelayChoice::Disabled => RelayMode::Disabled,
        RelayChoice::Pinned => RelayMode::custom(pinned_ladder()),
        RelayChoice::Custom(ladder) => RelayMode::custom(ladder.iter().cloned()),
    }
}

fn pinned_ladder() -> Vec<fofoca::iroh::RelayUrl> {
    fofoca::net::relay_ladder(&fofoca::protocol::RelayChoice::Pinned)
}

fn err(context: &str, error: &impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&format!("{context}: {error}"))
}

fn js_stage(context: &str, error: &JsValue) -> JsValue {
    JsValue::from_str(&format!("{context}: {error:?}"))
}

#[cfg(test)]
mod tests {
    use agent_inject_proto::lookup::{LookupOpts, RelayChoice};
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{InjectClient, pinned_ladder, relay_mode, signal_relay_mode};

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
        assert!(
            relay_mode(&LookupOpts::loopback().relay)
                .relay_map()
                .urls::<Vec<fofoca::iroh::RelayUrl>>()
                .is_empty()
        );
    }

    #[test]
    fn a_relay_override_replaces_the_ticket_ladder() {
        let url = "https://relay.example/".to_owned();
        let home = signal_relay_mode(&RelayChoice::Pinned, std::slice::from_ref(&url)).unwrap();
        let offered: Vec<fofoca::iroh::RelayUrl> = home.relay_map().urls();
        assert_eq!(offered, vec![url.parse().unwrap()]);

        let default = signal_relay_mode(&RelayChoice::Pinned, &[]).unwrap();
        assert_eq!(
            default
                .relay_map()
                .urls::<Vec<fofoca::iroh::RelayUrl>>()
                .len(),
            pinned_ladder().len()
        );
        assert!(signal_relay_mode(&RelayChoice::Pinned, &["not a url".to_owned()]).is_err());
    }

    #[test]
    fn parse_ticket_rejects_garbage() {
        assert!(InjectClient::parse_ticket("not-a-ticket").is_err());
    }

    #[test]
    fn parse_ticket_returns_what_the_session_accepts() {
        let ticket = agent_inject_proto::InjectTicket {
            addr: fofoca::iroh::EndpointAddr::new(
                fofoca::iroh::SecretKey::from_bytes(&[3u8; 32]).public(),
            ),
            secret: [5u8; 32],
            lookups: agent_inject_proto::lookup::LookupOpts::loopback(),
            accept: agent_inject_proto::Accept::Images,
        };
        assert_eq!(
            InjectClient::parse_ticket(&ticket.encode()).unwrap(),
            "images"
        );
    }
}
