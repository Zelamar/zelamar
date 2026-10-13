//! What travels *inside* an MLS application message.
//!
//! Content is CBOR-encoded: compact, self-describing and evolvable. A future
//! client may add fields or body kinds; older clients ignore unknown fields
//! and surface unknown kinds as such instead of failing the whole chat.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// Current content format version.
pub const CONTENT_VERSION: u16 = 0;

/// Largest encoded content accepted (64 KiB). Long texts are rare; anything
/// bigger is an attachment.
pub const MAX_ENCODED: usize = 64 * 1024;

/// Longest text body, in bytes of UTF-8.
pub const MAX_TEXT: usize = 32 * 1024;

/// Sender-chosen unique id of a message, used by replies, edits, reactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(#[serde(with = "serde_bytes")] pub [u8; 16]);

/// One message as the author wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppMessage {
    /// Content format version.
    #[serde(rename = "v")]
    pub version: u16,
    /// Unique id chosen by the author.
    #[serde(rename = "i")]
    pub id: MessageId,
    /// When the author sent it: UTC milliseconds since the Unix epoch,
    /// author's clock (ADR-0004). Never the receiver's clock.
    #[serde(rename = "t")]
    pub sent_at_ms: u64,
    /// The payload.
    #[serde(rename = "b")]
    pub body: Body,
}

/// The kinds of content. More arrive with later slices: reply, edit,
/// reaction, delete, receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Body {
    /// Plain text.
    Text {
        /// UTF-8 text, at most [`MAX_TEXT`] bytes.
        text: String,
    },
}

impl AppMessage {
    /// A new text message.
    #[must_use]
    pub fn text(id: MessageId, sent_at_ms: u64, text: impl Into<String>) -> Self {
        Self {
            version: CONTENT_VERSION,
            id,
            sent_at_ms,
            body: Body::Text { text: text.into() },
        }
    }

    /// Serializes the message.
    ///
    /// # Errors
    ///
    /// [`CoreError::TooLarge`] if a limit is exceeded.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).map_err(|e| CoreError::Encoding {
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
        if bytes.len() > MAX_ENCODED {
            return Err(CoreError::TooLarge {
                what: "message content",
                size: bytes.len(),
                limit: MAX_ENCODED,
            });
        }
        let message: Self = ciborium::from_reader(bytes)
            .map_err(|_| CoreError::malformed("message content", "invalid CBOR or shape"))?;
        if message.version > CONTENT_VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: "message content",
                version: message.version,
            });
        }
        message.validate()?;
        Ok(message)
    }

    const fn validate(&self) -> Result<()> {
        match &self.body {
            Body::Text { text } if text.len() > MAX_TEXT => Err(CoreError::TooLarge {
                what: "message text",
                size: text.len(),
                limit: MAX_TEXT,
            }),
            Body::Text { .. } => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn text_round_trips() {
        let message = AppMessage::text(MessageId([3; 16]), 1_760_000_000_000, "Hello, Zəlamar 🌊");
        let decoded = AppMessage::decode(&message.encode().expect("encode")).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn oversized_text_is_refused() {
        let message = AppMessage::text(MessageId([0; 16]), 0, "x".repeat(MAX_TEXT + 1));
        assert!(matches!(message.encode(), Err(CoreError::TooLarge { .. })));
    }

    #[test]
    fn future_versions_are_reported_not_misread() {
        let mut message = AppMessage::text(MessageId([0; 16]), 0, "hi");
        message.version = CONTENT_VERSION + 1;
        let mut bytes = Vec::new();
        ciborium::into_writer(&message, &mut bytes).expect("encode");
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
    }
}
