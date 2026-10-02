//! Where the web app lives.

/// Origin of the hosted web app. `AGENT_INJECT_WEB_ORIGIN` overrides it, for
/// a dev server or a tunnel.
const WEB_ORIGIN: &str = "https://inject.agent-habilis.com";

/// The page a phone opens to send files into the session.
pub(crate) fn web_url(ticket: &str) -> String {
    let origin = std::env::var("AGENT_INJECT_WEB_ORIGIN").unwrap_or_else(|_| WEB_ORIGIN.to_owned());
    format!("{}/app/inject/{ticket}", origin.trim_end_matches('/'))
}
