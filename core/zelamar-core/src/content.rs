//! What travels *inside* an MLS application message (`protocol/content.md`).
//!
//! Content is one CBOR map: version, message id, author's timestamp, a body
//! kind (`k`) and the kind's fields (`c`). Kinds are decoded in two steps so
//! that a kind from a newer Zelamar surfaces as [`Body::Unknown`] instead of
//! failing the whole chat, and unknown fields inside known kinds are ignored.

use openmls_rust_crypto::RustCrypto;
use openmls_traits::random::OpenMlsRand;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Current content format version.
pub const CONTENT_VERSION: u16 = 0;

/// Largest encoded content accepted (64 KiB). Long texts are rare; anything
/// bigger is an attachment.
pub const MAX_ENCODED: usize = 64 * 1024;

/// Longest text body, in bytes of UTF-8.
pub const MAX_TEXT: usize = 32 * 1024;

/// Longest reaction (one emoji or short grapheme cluster), in bytes.
pub const MAX_REACTION: usize = 32;

/// Most message ids one receipt may acknowledge.
pub const MAX_RECEIPT_IDS: usize = 256;

/// Longest body kind name accepted from the wire.
const MAX_KIND: usize = 32;

/// Sender-chosen unique id of a message, used by replies, edits, reactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(#[serde(with = "serde_bytes")] pub [u8; 16]);

impl MessageId {
    /// Draws a fresh random message id.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] if the system random generator fails.
    pub fn random() -> Result<Self> {
        RustCrypto::default()
            .random_array()
            .map(Self)
            .map_err(|e| CoreError::crypto("drawing a message id", e))
    }
}

/// How far a message has got on the receiver's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    /// Stored on the receiver's device.
    Delivered,
    /// Shown to the receiver.
    Read,
}

/// The kinds of content.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Body {
    /// Plain text.
    Text {
        /// UTF-8 text, at most [`MAX_TEXT`] bytes.
        text: String,
    },
    /// Text answering an earlier message.
    Reply {
        /// The message answered.
        to: MessageId,
        /// UTF-8 text, at most [`MAX_TEXT`] bytes.
        text: String,
    },
    /// A new version of one of the author's own earlier messages.
    Edit {
        /// The message edited.
        target: MessageId,
        /// The replacement text.
        text: String,
    },
    /// Sets or clears the author's reaction to a message.
    React {
        /// The message reacted to.
        target: MessageId,
        /// The reaction, or `None` to remove it.
        reaction: Option<String>,
    },
    /// Deletes one of the author's own earlier messages for everyone.
    Delete {
        /// The message deleted.
        target: MessageId,
    },
    /// Acknowledges messages as delivered or read.
    Receipt {
        /// How far they got.
        status: ReceiptStatus,
        /// The acknowledged messages, at most [`MAX_RECEIPT_IDS`].
        messages: Vec<MessageId>,
    },
    /// Who the author is, as they would be found in the directory. Sent
    /// once per chat, before the author's first message there; never shown
    /// as a message. Receivers treat the name as a claim until the directory
    /// confirms it.
    Profile {
        /// The author's username, normalized.
        username: String,
    },
    /// A kind this Zelamar does not know yet (sent by a newer version).
    /// Received only; never sent.
    Unknown {
        /// The kind's name, for diagnostics.
        kind: String,
    },
}

/// One message as the author wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMessage {
    /// Content format version.
    pub version: u16,
    /// Unique id chosen by the author.
    pub id: MessageId,
    /// When the author sent it: UTC milliseconds since the Unix epoch,
    /// author's clock (ADR-0004). Never the receiver's clock.
    pub sent_at_ms: u64,
    /// The payload.
    pub body: Body,
}

/// The CBOR shape of a message on the wire.
#[derive(Serialize, Deserialize)]
struct Wire {
    v: u16,
    i: MessageId,
    t: u64,
    k: String,
    c: ciborium::Value,
}

#[derive(Serialize, Deserialize)]
struct TextFields {
    text: String,
}

#[derive(Serialize, Deserialize)]
struct ReplyFields {
    to: MessageId,
    text: String,
}

#[derive(Serialize, Deserialize)]
struct EditFields {
    target: MessageId,
    text: String,
}

#[derive(Serialize, Deserialize)]
struct ReactFields {
    target: MessageId,
    reaction: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct DeleteFields {
    target: MessageId,
}

#[derive(Serialize, Deserialize)]
struct ProfileFields {
    username: String,
}

#[derive(Serialize, Deserialize)]
struct ReceiptFields {
    status: ReceiptStatus,
    messages: Vec<MessageId>,
}

impl AppMessage {
    /// A message with `body`, in the current content version.
    #[must_use]
    pub const fn new(id: MessageId, sent_at_ms: u64, body: Body) -> Self {
        Self {
            version: CONTENT_VERSION,
            id,
            sent_at_ms,
            body,
        }
    }

    /// A new text message.
    #[must_use]
    pub fn text(id: MessageId, sent_at_ms: u64, text: impl Into<String>) -> Self {
        Self::new(id, sent_at_ms, Body::Text { text: text.into() })
    }

    /// Serializes the message.
    ///
    /// # Errors
    ///
    /// [`CoreError::TooLarge`] if a limit is exceeded, or
    /// [`CoreError::Malformed`] for [`Body::Unknown`], which is never sent.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let (kind, fields) = match &self.body {
            Body::Text { text } => ("text", fields(&TextFields { text: text.clone() })?),
            Body::Reply { to, text } => (
                "reply",
                fields(&ReplyFields {
                    to: *to,
                    text: text.clone(),
                })?,
            ),
            Body::Edit { target, text } => (
                "edit",
                fields(&EditFields {
                    target: *target,
                    text: text.clone(),
                })?,
            ),
            Body::React { target, reaction } => (
                "react",
                fields(&ReactFields {
                    target: *target,
                    reaction: reaction.clone(),
                })?,
            ),
            Body::Delete { target } => ("delete", fields(&DeleteFields { target: *target })?),
            Body::Receipt { status, messages } => (
                "receipt",
                fields(&ReceiptFields {
                    status: *status,
                    messages: messages.clone(),
                })?,
            ),
            Body::Profile { username } => (
                "profile",
                fields(&ProfileFields {
                    username: username.clone(),
                })?,
            ),
            Body::Unknown { .. } => {
                return Err(CoreError::malformed(
                    "message content",
                    "unknown kinds are never sent",
                ));
            }
        };
        let wire = Wire {
            v: self.version,
            i: self.id,
            t: self.sent_at_ms,
            k: kind.to_owned(),
            c: fields,
        };
        let mut out = Vec::new();
        ciborium::into_writer(&wire, &mut out).map_err(|e| CoreError::Encoding {
            what: "message content",
            source: e.into(),
        })?;
        if out.len() > MAX_ENCODED {
            return Err(CoreError::TooLarge {
                what: "message content",
                size: out.len(),
                limit: MAX_ENCODED,
            });
        }
        Ok(out)
    }

    /// Parses and validates received content.
    ///
    /// # Errors
    ///
    /// [`CoreError::TooLarge`], [`CoreError::Malformed`] or
    /// [`CoreError::UnsupportedVersion`].
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "message content";
        if bytes.len() > MAX_ENCODED {
            return Err(CoreError::TooLarge {
                what: WHAT,
                size: bytes.len(),
                limit: MAX_ENCODED,
            });
        }
        let wire: Wire = ciborium::from_reader(bytes)
            .map_err(|_| CoreError::malformed(WHAT, "invalid CBOR or shape"))?;
        if wire.v > CONTENT_VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: WHAT,
                version: wire.v,
            });
        }
        if wire.k.len() > MAX_KIND {
            return Err(CoreError::malformed(WHAT, "kind name too long"));
        }
        let body = match wire.k.as_str() {
            "text" => {
                let f: TextFields = parse(&wire.c)?;
                Body::Text { text: f.text }
            }
            "reply" => {
                let f: ReplyFields = parse(&wire.c)?;
                Body::Reply {
                    to: f.to,
                    text: f.text,
                }
            }
            "edit" => {
                let f: EditFields = parse(&wire.c)?;
                Body::Edit {
                    target: f.target,
                    text: f.text,
                }
            }
            "react" => {
                let f: ReactFields = parse(&wire.c)?;
                Body::React {
                    target: f.target,
                    reaction: f.reaction,
                }
            }
            "delete" => {
                let f: DeleteFields = parse(&wire.c)?;
                Body::Delete { target: f.target }
            }
            "receipt" => {
                let f: ReceiptFields = parse(&wire.c)?;
                Body::Receipt {
                    status: f.status,
                    messages: f.messages,
                }
            }
            "profile" => {
                let f: ProfileFields = parse(&wire.c)?;
                Body::Profile {
                    username: f.username,
                }
            }
            other => Body::Unknown {
                kind: other.to_owned(),
            },
        };
        let message = Self {
            version: wire.v,
            id: wire.i,
            sent_at_ms: wire.t,
            body,
        };
        message.validate()?;
        Ok(message)
    }

    fn validate(&self) -> Result<()> {
        let too_large = |what, size, limit| Err(CoreError::TooLarge { what, size, limit });
        match &self.body {
            Body::Text { text } | Body::Reply { text, .. } | Body::Edit { text, .. }
                if text.len() > MAX_TEXT =>
            {
                too_large("message text", text.len(), MAX_TEXT)
            }
            Body::React {
                reaction: Some(reaction),
                ..
            } if reaction.is_empty() => Err(CoreError::malformed(
                "reaction",
                "empty; use None to remove a reaction",
            )),
            Body::React {
                reaction: Some(reaction),
                ..
            } if reaction.len() > MAX_REACTION => {
                too_large("reaction", reaction.len(), MAX_REACTION)
            }
            Body::Receipt { messages, .. } if messages.is_empty() => {
                Err(CoreError::malformed("receipt", "acknowledges nothing"))
            }
            Body::Receipt { messages, .. } if messages.len() > MAX_RECEIPT_IDS => {
                too_large("receipt", messages.len(), MAX_RECEIPT_IDS)
            }
            Body::Profile { username }
                if crate::service::normalize_username(username).ok().as_ref() != Some(username) =>
            {
                Err(CoreError::malformed(
                    "profile",
                    "username is not a normalized username",
                ))
            }
            _ => Ok(()),
        }
    }
}

fn fields<T: Serialize>(value: &T) -> Result<ciborium::Value> {
    ciborium::Value::serialized(value).map_err(|e| CoreError::Encoding {
        what: "message content",
        source: e.into(),
    })
}

fn parse<T: DeserializeOwned>(value: &ciborium::Value) -> Result<T> {
    value
        .deserialized()
        .map_err(|_| CoreError::malformed("message content", "fields do not match the kind"))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const A: MessageId = MessageId([0xA; 16]);
    const B: MessageId = MessageId([0xB; 16]);

    fn round_trip(body: Body) {
        let message = AppMessage::new(MessageId([3; 16]), 1_760_000_000_000, body);
        let decoded = AppMessage::decode(&message.encode().expect("encode")).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn every_kind_round_trips() {
        round_trip(Body::Text {
            text: "Hello, Zəlamar 🌊".into(),
        });
        round_trip(Body::Reply {
            to: A,
            text: "By the pier".into(),
        });
        round_trip(Body::Edit {
            target: A,
            text: "See you at seven".into(),
        });
        round_trip(Body::React {
            target: A,
            reaction: Some("❤️".into()),
        });
        round_trip(Body::React {
            target: A,
            reaction: None,
        });
        round_trip(Body::Delete { target: B });
        round_trip(Body::Receipt {
            status: ReceiptStatus::Read,
            messages: vec![A, B],
        });
    }

    #[test]
    fn a_kind_from_a_newer_zelamar_is_surfaced_not_fatal() {
        #[derive(Serialize)]
        struct Poll {
            question: String,
        }
        let wire = Wire {
            v: 0,
            i: A,
            t: 1,
            k: "poll".into(),
            c: fields(&Poll {
                question: "Pier or café?".into(),
            })
            .expect("fields"),
        };
        let mut bytes = Vec::new();
        ciborium::into_writer(&wire, &mut bytes).expect("encode");
        let decoded = AppMessage::decode(&bytes).expect("decode");
        assert_eq!(
            decoded.body,
            Body::Unknown {
                kind: "poll".into()
            }
        );
        assert!(decoded.encode().is_err(), "unknown kinds are never re-sent");
    }

    #[test]
    fn limits_are_enforced() {
        let at = |body| AppMessage::new(A, 0, body);
        assert!(matches!(
            at(Body::Text {
                text: "x".repeat(MAX_TEXT + 1)
            })
            .encode(),
            Err(CoreError::TooLarge { .. })
        ));
        assert!(
            at(Body::React {
                target: A,
                reaction: Some(String::new())
            })
            .encode()
            .is_err()
        );
        assert!(
            at(Body::React {
                target: A,
                reaction: Some("🙂".repeat(20))
            })
            .encode()
            .is_err()
        );
        assert!(
            at(Body::Receipt {
                status: ReceiptStatus::Delivered,
                messages: vec![]
            })
            .encode()
            .is_err()
        );
        assert!(
            at(Body::Receipt {
                status: ReceiptStatus::Delivered,
                messages: vec![A; MAX_RECEIPT_IDS + 1]
            })
            .encode()
            .is_err()
        );
    }

    #[test]
    fn fields_of_the_wrong_kind_are_malformed() {
        let wire = Wire {
            v: 0,
            i: A,
            t: 1,
            k: "reply".into(),
            c: fields(&TextFields {
                text: "no target".into(),
            })
            .expect("fields"),
        };
        let mut bytes = Vec::new();
        ciborium::into_writer(&wire, &mut bytes).expect("encode");
        assert!(matches!(
            AppMessage::decode(&bytes),
            Err(CoreError::Malformed { .. })
        ));
    }

    #[test]
    fn future_versions_are_reported_not_misread() {
        let mut bytes = Vec::new();
        let wire = Wire {
            v: CONTENT_VERSION + 1,
            i: A,
            t: 0,
            k: "text".into(),
            c: fields(&TextFields { text: "hi".into() }).expect("fields"),
        };
        ciborium::into_writer(&wire, &mut bytes).expect("encode");
        assert!(matches!(
            AppMessage::decode(&bytes),
            Err(CoreError::UnsupportedVersion { .. })
        ));
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = AppMessage::decode(&bytes);
        }

        #[test]
        fn any_text_round_trips(text in ".{0,200}", t in any::<u64>(), id in any::<[u8; 16]>()) {
            let message = AppMessage::text(MessageId(id), t, text);
            let decoded = AppMessage::decode(&message.encode().expect("encode")).expect("decode");
            prop_assert_eq!(decoded, message);
        }
    }
}
