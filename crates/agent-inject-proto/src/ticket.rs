//! The inject ticket — the whole capability to send files into one session,
//! in one string.

use anyhow::{Context, Result, bail};
use fofoca_protocol::iroh_base::EndpointAddr;

use crate::framing::SECRET_LEN;
use crate::lookup::LookupOpts;
use crate::peer_addr::{endpoint_addr_from_json, endpoint_addr_to_json};
use crate::token::{self, TokenType};

/// A decoded inject ticket: the bearer secret, the session's relay config,
/// and the receiver's address.
///
/// Payload layout: `secret(32) ‖ lookups ‖ addr_len(u16 LE) ‖ addr_json ‖
/// accept(1)`.
/// Every field is self-delimiting, so a field can be appended later; a
/// decoder ignores trailing bytes it does not know.
///
/// The secret is a pure bearer capability: whoever holds this string can
/// write files into the session. The web app puts it in the URL path
/// (`/app/inject/<ticket>`). The ticket embeds the receiver's live address,
/// so it dies when the receiver stops.
#[derive(Debug, Clone)]
pub struct InjectTicket {
    pub addr: EndpointAddr,
    pub secret: [u8; SECRET_LEN],
    pub lookups: LookupOpts,
    pub accept: Accept,
}

/// What the session takes. The page shows only the matching pickers, and the
/// receiver refuses the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accept {
    #[default]
    Any,
    Images,
}

impl Accept {
    /// Stable lowercase name, used by the web page.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Accept::Any => "any",
            Accept::Images => "images",
        }
    }

    const fn to_byte(self) -> u8 {
        match self {
            Accept::Any => 0,
            Accept::Images => 1,
        }
    }

    fn from_byte(byte: u8) -> Result<Self> {
        Ok(match byte {
            0 => Accept::Any,
            1 => Accept::Images,
            other => bail!("unknown accept mode in ticket: {other}"),
        })
    }
}

impl InjectTicket {
    /// Encode as a token (`type = inject`).
    ///
    /// # Panics
    /// If the embedded [`LookupOpts`] exceeds its wire bounds — see
    /// [`LookupOpts::encode_into`].
    #[must_use]
    pub fn encode(&self) -> String {
        let mut payload = Vec::with_capacity(SECRET_LEN + 1 + 128);
        payload.extend_from_slice(&self.secret);
        self.lookups.encode_into(&mut payload);
        let addr_json = serde_json::to_vec(&endpoint_addr_to_json(&self.addr))
            .expect("EndpointAddr JSON always serializes");
        payload.extend_from_slice(
            &u16::try_from(addr_json.len())
                .expect("an EndpointAddr JSON is orders of magnitude under 64 KiB")
                .to_le_bytes(),
        );
        payload.extend_from_slice(&addr_json);
        payload.push(self.accept.to_byte());
        token::encode(TokenType::Inject, &payload)
    }

    /// Decode an inject ticket.
    ///
    /// # Errors
    /// Not a valid token, the wrong token type, or a malformed payload.
    pub fn decode(ticket: &str) -> Result<Self> {
        let (token_type, payload) = token::decode(ticket.trim())?;
        if token_type != TokenType::Inject {
            bail!("not an inject ticket (got a {token_type:?} token)");
        }
        let secret_bytes = payload.get(..SECRET_LEN).context("ticket too short")?;
        let mut secret = [0u8; SECRET_LEN];
        secret.copy_from_slice(secret_bytes);
        let mut pos = SECRET_LEN;
        let lookups = LookupOpts::decode_from(&payload, &mut pos)?;
        let addr_len = usize::from(crate::lookup::read_u16(&payload, &mut pos)?);
        let end = pos
            .checked_add(addr_len)
            .context("address length overflow")?;
        let addr_raw = payload.get(pos..end).context("ticket address truncated")?;
        let json: serde_json::Value =
            serde_json::from_slice(addr_raw).context("ticket address is not JSON")?;
        let (_, addr) = endpoint_addr_from_json(&json)?;
        // Tickets from before `accept` existed end at the address.
        let accept = payload
            .get(end)
            .copied()
            .map_or(Ok(Accept::Any), Accept::from_byte)?;
        Ok(Self {
            addr,
            secret,
            lookups,
            accept,
        })
    }
}

#[cfg(test)]
mod tests {
    use fofoca_protocol::iroh_base::{EndpointAddr, SecretKey};

    use super::{Accept, InjectTicket};
    use crate::lookup::{LookupOpts, RelayChoice};
    use crate::token::{self, TokenType};

    fn fixed_addr() -> EndpointAddr {
        let key = SecretKey::from_bytes(&[3u8; 32]);
        EndpointAddr::new(key.public())
            .with_ip_addr("127.0.0.1:4433".parse().unwrap())
            .with_relay_url("https://relay.example/".parse().unwrap())
    }

    fn ticket(lookups: LookupOpts) -> InjectTicket {
        InjectTicket {
            addr: fixed_addr(),
            secret: [5u8; 32],
            lookups,
            accept: Accept::Any,
        }
    }

    #[test]
    fn round_trips_each_lookup_shape() {
        for lookups in [
            LookupOpts::loopback(),
            LookupOpts::public_preset(),
            LookupOpts {
                mdns: false,
                dht: false,
                relay: RelayChoice::Custom(vec!["https://a.example".parse().unwrap()]),
            },
        ] {
            let original = ticket(lookups);
            let decoded = InjectTicket::decode(&original.encode()).unwrap();
            assert_eq!(decoded.secret, original.secret);
            assert_eq!(decoded.lookups, original.lookups);
            assert_eq!(decoded.addr, original.addr);
        }
    }

    #[test]
    fn encoding_is_pinned() {
        // Golden vector: a change here is a wire break and must be deliberate.
        let encoded = ticket(LookupOpts::loopback()).encode();
        let (kind, payload) = token::decode(&encoded).unwrap();
        assert_eq!(kind, TokenType::Inject);
        assert_eq!(payload[..32], [5u8; 32]);
        assert_eq!(payload[32], 0, "loopback lookup flags");
        let addr_len = usize::from(u16::from_le_bytes([payload[33], payload[34]]));
        assert_eq!(payload.len(), 36 + addr_len);
        assert_eq!(payload[35 + addr_len], 0, "accept any");
        assert_eq!(
            encoded,
            ticket(LookupOpts::loopback()).encode(),
            "deterministic"
        );
    }

    #[test]
    fn accept_round_trips() {
        for accept in [Accept::Any, Accept::Images] {
            let original = InjectTicket {
                accept,
                ..ticket(LookupOpts::loopback())
            };
            let decoded = InjectTicket::decode(&original.encode()).unwrap();
            assert_eq!(decoded.accept, accept);
        }
    }

    #[test]
    fn accept_labels_are_pinned() {
        assert_eq!(Accept::Any.label(), "any");
        assert_eq!(Accept::Images.label(), "images");
    }

    #[test]
    fn a_ticket_without_accept_takes_any() {
        let original = InjectTicket {
            accept: Accept::Images,
            ..ticket(LookupOpts::loopback())
        };
        let (_, mut payload) = token::decode(&original.encode()).unwrap();
        let addr_len = usize::from(u16::from_le_bytes([payload[33], payload[34]]));
        payload.truncate(35 + addr_len);
        let older = token::encode(TokenType::Inject, &payload);
        assert_eq!(InjectTicket::decode(&older).unwrap().accept, Accept::Any);
    }

    #[test]
    fn rejects_an_unknown_accept() {
        let (_, mut payload) = token::decode(&ticket(LookupOpts::loopback()).encode()).unwrap();
        *payload.last_mut().unwrap() = 9;
        let error = InjectTicket::decode(&token::encode(TokenType::Inject, &payload))
            .unwrap_err()
            .to_string();
        assert!(error.contains("accept"), "{error}");
    }

    #[test]
    fn ignores_trailing_bytes() {
        let (_, mut payload) = token::decode(&ticket(LookupOpts::loopback()).encode()).unwrap();
        payload.push(0xaa);
        let extended = token::encode(TokenType::Inject, &payload);
        assert!(InjectTicket::decode(&extended).is_ok());
    }

    #[test]
    fn rejects_another_token_type() {
        let (_, payload) = token::decode(&ticket(LookupOpts::loopback()).encode()).unwrap();
        let mount = token::encode(TokenType::Mount, &payload);
        let error = InjectTicket::decode(&mount).unwrap_err().to_string();
        assert!(error.contains("not an inject ticket"), "{error}");
    }

    #[test]
    fn rejects_truncated_address() {
        let (_, mut payload) = token::decode(&ticket(LookupOpts::loopback()).encode()).unwrap();
        payload.truncate(payload.len() - 3);
        assert!(InjectTicket::decode(&token::encode(TokenType::Inject, &payload)).is_err());
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let encoded = format!("  {}\n", ticket(LookupOpts::loopback()).encode());
        assert!(InjectTicket::decode(&encoded).is_ok());
    }
}
