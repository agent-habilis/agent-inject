//! The WebRTC lane: how a browser, which has no UDP socket and cannot speak
//! QUIC directly, reaches the receiver. Copied from agent-share's
//! `mount/webrtc.rs`.
//!
//! Two connections, not one, and the split is load-bearing. iroh only fans a
//! connect's Initial out to candidate paths **while the remote has no selected
//! path**, so a live connection cannot be upgraded onto a newly attached
//! transport in place. So:
//!
//! 1. A short-lived connection on [`WEBRTC_SIGNAL_ALPN`] carries one JSEP
//!    envelope each way, then closes. Over the relay is the interesting case:
//!    it is the browser's only way to reach a receiver behind NAT.
//! 2. The sender then opens a **fresh** connection on the upload ALPN against
//!    an address containing *only* the WebRTC custom addr, so the Initial fans
//!    out over the data channel.
//!
//! The relay is a rendezvous, not a transport: it carries the SDP exchange and
//! never a byte of file data.
//!
//! A session registry holds at most one session per peer. Both roles ask it
//! before negotiating and defer to it afterwards, so a lost race reuses the
//! session that won instead of failing.

use agent_inject_proto::WEBRTC_SIGNAL_ALPN;
use anyhow::{Context, Result};
use fofoca::iroh::endpoint::Connection;
use fofoca::iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use fofoca_iroh_webrtc_transport::{
    IceConfig, MAX_ENVELOPE_BYTES, NegotiatedSession, SignalEnvelope, WebRtcHandle, answer_with,
    custom_addr, offer_with,
};

/// How long to let a JSEP negotiation run before giving up.
///
/// Generous because it covers STUN gathering on both sides plus the
/// DTLS/SCTP handshake, and a false timeout costs the whole connection.
const JSEP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

/// An address that reaches `remote` over the data channel and nothing else.
///
/// Listing IP or relay paths alongside it would let the upload connection pick
/// one, which is the whole thing this lane exists to avoid.
#[must_use]
pub fn webrtc_only_addr(remote: EndpointId) -> EndpointAddr {
    EndpointAddr::from_parts(remote, [TransportAddr::Custom(custom_addr(remote))])
}

/// Every path on `conn`, `*` marking the selected one.
pub(crate) fn path_summary(conn: &Connection) -> Vec<String> {
    conn.paths()
        .iter()
        .map(|path| {
            let kind = if path.is_relay() {
                "relay"
            } else if path.is_ip() {
                "ip"
            } else if matches!(
                path.remote_addr(),
                TransportAddr::Custom(addr)
                    if addr.id() == fofoca_iroh_webrtc_transport::WEBRTC_TRANSPORT_ID
            ) {
                "webrtc"
            } else {
                "other"
            };
            if path.is_selected() {
                format!("*{kind}")
            } else {
                kind.to_owned()
            }
        })
        .collect()
}

/// Poll until `satisfied` holds for `conn`, or fail with `describe`.
///
/// Selection is not settled when `connect` resolves — a fresh connection has no
/// selected path at all for the first moments — so this waits rather than
/// sampling once. The predicate takes the whole connection because iroh does
/// not export a nameable type for a single path.
pub(crate) async fn ensure_selected<S, D>(
    conn: &Connection,
    satisfied: S,
    describe: D,
) -> Result<()>
where
    S: Fn(&Connection) -> bool,
    D: Fn() -> String,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if satisfied(conn) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("{}", describe());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Require the upload's *selected* path to be the `WebRTC` custom transport.
///
/// The check that was missing everywhere. Registering the transport and dialling
/// a WebRTC-only address does not make the connection use it: iroh merges that
/// address into a book that already holds the receiver's relay (the JSEP dial
/// put it there) and the warm path can answer first. A caller that then reports
/// "webrtc" is reporting its intent, not the wire.
///
/// # Errors
/// No WebRTC path is selected within five seconds.
pub async fn ensure_webrtc_selected(conn: &Connection, whose: &str) -> Result<()> {
    ensure_selected(
        conn,
        |conn| {
            conn.paths().iter().any(|path| {
                path.is_selected()
                    && matches!(
                        path.remote_addr(),
                        TransportAddr::Custom(addr)
                            if addr.id() == fofoca_iroh_webrtc_transport::WEBRTC_TRANSPORT_ID
                    )
            })
        },
        || {
            format!(
                "{whose} selected a non-WebRTC path (paths={:?}); the upload rode \
                 another transport",
                path_summary(conn)
            )
        },
    )
    .await
}

/// Attach `session`, or accept the one another lane attached first.
///
/// A duplicate is not a failure: the registry already holds a usable channel to
/// `remote`, which is all the upload needs. The session we just negotiated is
/// dropped, and the registry aborts its driver on that path, so nothing leaks.
///
/// Keyed on the registry's *state* rather than on the error's identity — the
/// state is what licenses the reuse, and it stays correct even if the attach
/// failed for a reason nobody anticipated.
fn attach_or_reuse(
    handle: &WebRtcHandle,
    remote: EndpointId,
    session: NegotiatedSession,
) -> Result<()> {
    match handle.attach(remote, session) {
        Ok(()) => Ok(()),
        Err(error) if handle.has_session(&remote) => {
            tracing::debug!(%remote, %error, "another lane attached first; reusing that session");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Serve one inbound signalling connection: read the offer, answer it, attach
/// the resulting session to the transport.
///
/// The envelope's `endpoint_id` is **ignored** in favour of
/// `connection.remote_id()`. Over an authenticated carrier the TLS-proven
/// identity is the real one, and trusting the claim instead would let a peer
/// attach a session under someone else's name.
///
/// # Errors
/// The stream carries no readable envelope, the SDP is unusable, or the
/// negotiation does not complete before [`JSEP_DEADLINE`].
pub(crate) async fn serve_signal(
    conn: &Connection,
    local: EndpointId,
    handle: &WebRtcHandle,
    ice: &IceConfig,
) -> Result<()> {
    let remote = conn.remote_id();
    let (mut send, mut recv) = conn.accept_bi().await.context("accept signal stream")?;
    let raw = recv
        .read_to_end(MAX_ENVELOPE_BYTES)
        .await
        .context("read signal offer")?;

    // Refuse before gathering, not after. A session already exists — the mesh
    // lane got here first — so a full STUN round would end in a refused attach
    // anyway. Say so now, so the offerer stops spending its ICE budget and
    // dials the channel it can already reach us on.
    //
    // Refuse rather than tear down and renegotiate: dropping a working mesh
    // session is strictly worse, and the upload does not
    // need its own session to begin with.
    if handle.has_session(&remote) {
        send.write_all(&serde_json::to_vec(&SignalEnvelope::error(
            "a WebRTC session with you already exists; dial the custom addr",
        ))?)
        .await
        .context("send signal refusal")?;
        send.finish().context("finish signal stream")?;
        // Wait for the acknowledgement before returning, because returning is
        // what closes the connection. The answer path below gets away without
        // this only by accident — it spends the next several seconds inside
        // `complete()`, which is long enough for the flush. A refusal has no
        // such pause, and the offerer saw `ConnectionLost` instead of the
        // reason, which is the failure mode the refusal exists to replace.
        let _ = send.stopped().await;
        tracing::debug!(%remote, "refused a duplicate signal round; a session already exists");
        return Ok(());
    }

    let offer: SignalEnvelope = serde_json::from_slice(&raw).context("parse signal offer")?;

    let (pending, answer) = answer_with(local, &offer, ice)
        .await
        .context("build WebRTC answer")?;
    send.write_all(&serde_json::to_vec(&answer)?)
        .await
        .context("send signal answer")?;
    send.finish().context("finish signal stream")?;

    // Boxed: the pending session carries the whole sans-io str0m state, which
    // is large enough to be worth keeping off this future's stack.
    let session = Box::pin(pending.complete(JSEP_DEADLINE))
        .await
        .context("complete WebRTC answer")?;
    // Tolerant of the narrow race where the other lane landed during our round.
    attach_or_reuse(handle, remote, session).context("attach session")?;
    tracing::debug!(%remote, "webrtc lane attached (answerer)");
    Ok(())
}

/// Negotiate a `WebRTC` session with `receiver` and return an address that
/// reaches it over the data channel and nothing else.
///
/// The returned address deliberately carries only the custom addr: handing
/// back one that also listed IP or relay paths would let the upload connection
/// pick those instead, which is exactly what this lane exists to avoid.
///
/// # Errors
/// The signalling dial fails, the receiver refuses, or the negotiation does
/// not complete before [`JSEP_DEADLINE`].
pub async fn dial_webrtc(
    endpoint: &Endpoint,
    receiver: EndpointAddr,
    handle: &WebRtcHandle,
    ice: &IceConfig,
) -> Result<EndpointAddr> {
    let remote = receiver.id;
    let local = endpoint.id();

    // The other lane may already have a channel to this peer. It is the same
    // registry and the same `custom_addr`, so there is nothing to negotiate —
    // skipping the round here is what turns a lost race from a 20s stall
    // ending in relay into a no-op.
    if handle.has_session(&remote) {
        tracing::debug!(%remote, "reusing the live WebRTC session; skipping JSEP");
        return Ok(webrtc_only_addr(remote));
    }

    let conn = endpoint
        .connect(receiver, WEBRTC_SIGNAL_ALPN)
        .await
        .context("dial the WebRTC signal ALPN")?;
    let (mut send, mut recv) = conn.open_bi().await.context("open signal stream")?;

    let (pending, offer) = offer_with(local, ice).await.context("build WebRTC offer")?;
    send.write_all(&serde_json::to_vec(&offer)?)
        .await
        .context("send signal offer")?;
    send.finish().context("finish signal stream")?;

    let raw = recv
        .read_to_end(MAX_ENVELOPE_BYTES)
        .await
        .context("read signal answer")?;
    let answer: SignalEnvelope = serde_json::from_slice(&raw).context("parse signal answer")?;

    // An explicit refusal, which the answerer now sends when it already holds a
    // session with us. Handled here rather than left to `complete`, which would
    // report it as "expected an answer envelope" and send the upload to relay.
    if let SignalEnvelope::Error { reason, .. } = &answer {
        conn.close(0u32.into(), b"jsep refused");
        if handle.has_session(&remote) {
            tracing::debug!(%remote, "receiver refused a duplicate round; using the session we hold");
            return Ok(webrtc_only_addr(remote));
        }
        anyhow::bail!("receiver refused WebRTC signalling: {reason}");
    }

    let session = Box::pin(pending.complete(&answer, JSEP_DEADLINE))
        .await
        .context("complete WebRTC offer")?;
    attach_or_reuse(handle, remote, session).context("attach session")?;

    // Signalling is done; the data rides its own connection.
    conn.close(0u32.into(), b"jsep done");
    tracing::debug!(%remote, "webrtc lane attached (offerer)");

    Ok(webrtc_only_addr(remote))
}
