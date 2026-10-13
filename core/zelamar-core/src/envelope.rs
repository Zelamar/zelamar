//! The transport-agnostic envelope (ADR-0004, `protocol/envelope.md`).
//!
//! An envelope is the unit every transport carries: today the delivery
//! service, tomorrow Drift (local network, Bluetooth, "bottles"). Its outer
//! header holds only what routing needs. There is no sender, no timestamp,
//! no chat id: those live encrypted inside the MLS payload.
//!
//! Layout, version 0 (all integers big-endian):
//!
//! ```text
//! magic "ZE" (2) | version (1) | envelope id (16) | mailbox id (32) |
//! payload length (4) | payload (MLS message, TLS-encoded)
//! ```

use openmls_rust_crypto::RustCrypto;
use openmls_traits::random::OpenMlsRand;

use crate::error::{CoreError, Result};
use crate::identity::Reader;

const MAGIC: [u8; 2] = *b"ZE";
const VERSION: u8 = 0;
const HEADER_LEN: usize = 2 + 1 + 16 + 32 + 4;

/// Largest payload accepted (1 MiB). Attachments travel separately.
pub const MAX_PAYLOAD: usize = 1 << 20;

/// Unique id of an envelope. Lets receivers drop duplicates that arrive via
/// more than one transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnvelopeId(pub [u8; 16]);

impl EnvelopeId {
    /// Draws a fresh random envelope id.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] if the system random generator fails.
    pub fn random() -> Result<Self> {
        RustCrypto::default()
            .random_array()
            .map(Self)
            .map_err(|e| CoreError::crypto("drawing an envelope id", e))
    }
}

/// Opaque address of a recipient device's mailbox. Not derived from the
/// device's identity, so it can rotate without breaking anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MailboxId(pub [u8; 32]);

/// One sealed message in transit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Deduplication id.
    pub id: EnvelopeId,
    /// Where to deliver it.
    pub to: MailboxId,
    /// The encrypted MLS message.
    pub payload: Vec<u8>,
}

impl Envelope {
    /// Serializes the envelope.
    ///
    /// # Errors
    ///
    /// [`CoreError::TooLarge`] if the payload exceeds [`MAX_PAYLOAD`].
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(CoreError::TooLarge {
                what: "envelope payload",
                size: self.payload.len(),
                limit: MAX_PAYLOAD,
            });
        }
        let len = u32::try_from(self.payload.len()).map_err(|_| CoreError::TooLarge {
            what: "envelope payload",
            size: self.payload.len(),
            limit: MAX_PAYLOAD,
        })?;
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&self.id.0);
        out.extend_from_slice(&self.to.0);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    /// Parses an envelope, rejecting anything that is not exactly one
    /// well-formed version 0 envelope.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`], [`CoreError::UnsupportedVersion`] or
    /// [`CoreError::TooLarge`].
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "envelope";
        let mut reader = Reader::new(bytes, WHAT);
        if reader.array::<2>()? != MAGIC {
            return Err(CoreError::malformed(WHAT, "not a Zelamar envelope"));
        }
        let version = reader.u8()?;
        if version != VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: WHAT,
                version: version.into(),
            });
        }
        let id = EnvelopeId(reader.array()?);
        let to = MailboxId(reader.array()?);
        let payload = reader.bytes_with_u32_len(MAX_PAYLOAD)?.to_vec();
        reader.finish()?;
        Ok(Self { id, to, payload })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn envelope(payload: Vec<u8>) -> Envelope {
        Envelope {
            id: EnvelopeId([1; 16]),
            to: MailboxId([2; 32]),
            payload,
        }
    }

    #[test]
    fn layout_matches_the_specification() {
        let bytes = envelope(vec![0xAB, 0xCD]).encode().expect("encode");
        assert_eq!(&bytes[..3], b"ZE\0");
        assert_eq!(&bytes[3..19], &[1; 16]);
        assert_eq!(&bytes[19..51], &[2; 32]);
        assert_eq!(&bytes[51..55], &2u32.to_be_bytes());
        assert_eq!(&bytes[55..], &[0xAB, 0xCD]);
    }

    #[test]
    fn rejects_foreign_magic_future_versions_and_trailing_bytes() {
        let good = envelope(vec![9]).encode().expect("encode");

        let mut foreign = good.clone();
        foreign[0] = b'X';
        assert!(matches!(
            Envelope::decode(&foreign),
            Err(CoreError::Malformed { .. })
        ));

        let mut future = good.clone();
        future[2] = 1;
        assert!(matches!(
            Envelope::decode(&future),
            Err(CoreError::UnsupportedVersion { version: 1, .. })
        ));

        let mut trailing = good;
        trailing.push(0);
        assert!(matches!(
            Envelope::decode(&trailing),
            Err(CoreError::Malformed { .. })
        ));
    }

    #[test]
    fn oversized_payloads_are_refused_both_ways() {
        let big = envelope(vec![0; MAX_PAYLOAD + 1]);
        assert!(matches!(big.encode(), Err(CoreError::TooLarge { .. })));

        let mut claims_huge = envelope(vec![]).encode().expect("encode");
        claims_huge[51..55].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            Envelope::decode(&claims_huge),
            Err(CoreError::TooLarge { .. })
        ));
    }

    proptest! {
        #[test]
        fn round_trips(id in any::<[u8; 16]>(), to in any::<[u8; 32]>(), payload in proptest::collection::vec(any::<u8>(), 0..512)) {
            let original = Envelope { id: EnvelopeId(id), to: MailboxId(to), payload };
            let decoded = Envelope::decode(&original.encode().expect("encode")).expect("decode");
            prop_assert_eq!(decoded, original);
        }

        #[test]
        fn never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = Envelope::decode(&bytes);
        }
    }
}
