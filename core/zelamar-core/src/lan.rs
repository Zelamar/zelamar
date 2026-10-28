//! Drift D1: handing envelopes to devices on the same local network while
//! the delivery service is out of reach (ADR-0020, `protocol/drift-lan.md`).
//!
//! The core decides what may travel and how a mailbox is asked about; the
//! app moves the bytes. A device learns which of its neighbours hosts a
//! mailbox only by tags that change with every nonce.

use hkdf::Hkdf;
use openmls::prelude::{MlsMessageBodyIn, MlsMessageIn, tls_codec::Deserialize as _};
use sha2::Sha256;

use crate::device::Device;
use crate::envelope::{Envelope, EnvelopeId, MailboxId};
use crate::error::{CoreError, Result};

/// Bytes of a listener's nonce.
pub const NONCE_LEN: usize = 16;

/// Bytes of a mailbox tag.
pub const TAG_LEN: usize = 16;

/// What a mailbox is called under one listener's nonce: it says nothing
/// about the mailbox to anyone who does not already know its id, and
/// changes with every nonce.
///
/// # Errors
///
/// [`CoreError::Crypto`] if the derivation fails, which it cannot for
/// these lengths.
pub fn tag(mailbox: &MailboxId, nonce: &[u8; NONCE_LEN]) -> Result<[u8; TAG_LEN]> {
    let kdf = Hkdf::<Sha256>::new(Some(nonce), &mailbox.0);
    let mut tag = [0u8; TAG_LEN];
    kdf.expand(b"zelamar/lan/v0", &mut tag)
        .map_err(|e| CoreError::crypto("deriving a mailbox tag", format!("{e:?}")))?;
    Ok(tag)
}

/// One envelope that may be handed to a neighbour hosting its mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parcel {
    /// The envelope's id: what the neighbour answers with once it has it.
    pub id: EnvelopeId,
    /// The mailbox it is for.
    pub mailbox: MailboxId,
    /// The encoded envelope (`protocol/envelope.md`).
    pub envelope: Vec<u8>,
}

impl Device {
    /// Everything in the outbox that may travel over the local network:
    /// messages, one envelope per recipient. Changes to a chat's members,
    /// and the welcomes that go with them, wait for the service, which
    /// orders them (ADR-0012). The outbox keeps every item until the
    /// service has it too.
    ///
    /// # Errors
    ///
    /// A storage or integrity error.
    pub fn parcels(&self) -> Result<Vec<Parcel>> {
        let mut parcels = Vec::new();
        for outgoing in self.outbox()? {
            if outgoing.commit.is_some() || !is_message(&outgoing.payload) {
                continue;
            }
            for mailbox in &outgoing.recipients {
                parcels.push(Parcel {
                    id: outgoing.id,
                    mailbox: *mailbox,
                    envelope: Envelope {
                        id: outgoing.id,
                        to: *mailbox,
                        payload: outgoing.payload.clone(),
                    }
                    .encode()?,
                });
            }
        }
        Ok(parcels)
    }

    /// This device's own mailbox tag under a neighbour's `nonce`.
    ///
    /// # Errors
    ///
    /// As [`tag`].
    pub fn own_tag(&self, nonce: &[u8; NONCE_LEN]) -> Result<[u8; TAG_LEN]> {
        tag(&self.mailbox(), nonce)
    }
}

/// Whether `payload` is a message inside a chat, rather than a welcome
/// into one or a change to it.
fn is_message(payload: &[u8]) -> bool {
    MlsMessageIn::tls_deserialize_exact(payload)
        .is_ok_and(|message| matches!(message.extract(), MlsMessageBodyIn::PrivateMessage(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_differ_by_mailbox_and_by_nonce() {
        let (a, b) = (MailboxId([1; 32]), MailboxId([2; 32]));
        let (n1, n2) = ([7; NONCE_LEN], [8; NONCE_LEN]);
        let t = |m, n| tag(m, n).expect("tag");
        assert_eq!(t(&a, &n1), t(&a, &n1), "the same question, the same tag");
        assert_ne!(t(&a, &n1), t(&b, &n1));
        assert_ne!(t(&a, &n1), t(&a, &n2), "a new nonce, a new tag");
    }
}
