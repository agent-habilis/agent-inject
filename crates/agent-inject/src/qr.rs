//! The session URL as a terminal QR code, so a phone can scan it.

use qrcode::QrCode;
use qrcode::render::unicode::Dense1x2;

/// Two rows of modules per character line, with the quiet zone scanners need.
pub(crate) fn render(url: &str) -> Option<String> {
    let code = QrCode::new(url.as_bytes()).ok()?;
    Some(code.render::<Dense1x2>().quiet_zone(true).build())
}
