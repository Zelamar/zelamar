//! The core's error type.

use std::error::Error as StdError;

/// Boxed source error from a dependency (`OpenMLS`, CBOR, crypto backend).
pub type Source = Box<dyn StdError + Send + Sync + 'static>;

/// Everything that can go wrong inside Zelamar Core.
///
/// Variants describe *what* failed in Zelamar's terms; the underlying library
/// error, when there is one, is kept as the `source` for diagnostics.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// A cryptographic primitive failed (key generation, signing, randomness).
    #[error("cryptographic operation failed: {context}")]
    Crypto {
        /// What the core was doing.
        context: &'static str,
        /// The underlying error.
        #[source]
        source: Source,
    },

    /// The MLS layer rejected an operation or an incoming message.
    #[error("MLS operation failed: {context}")]
    Mls {
        /// What the core was doing.
        context: &'static str,
        /// The underlying error.
        #[source]
        source: Source,
    },

    /// Bytes from the outside world did not have the expected shape.
    #[error("malformed {what}: {reason}")]
    Malformed {
        /// The kind of data being parsed.
        what: &'static str,
        /// Why it was rejected.
        reason: &'static str,
    },

    /// Data declares a format version this core does not understand.
    #[error("unsupported {what} version {version}")]
    UnsupportedVersion {
        /// The kind of data.
        what: &'static str,
        /// The version found.
        version: u16,
    },

    /// Content could not be serialized.
    #[error("could not encode {what}")]
    Encoding {
        /// The kind of data.
        what: &'static str,
        /// The underlying error.
        #[source]
        source: Source,
    },

    /// Data exceeds a hard size limit (protection against resource abuse).
    #[error("{what} is too large: {size} bytes (limit {limit})")]
    TooLarge {
        /// The kind of data.
        what: &'static str,
        /// Actual size in bytes.
        size: usize,
        /// Maximum allowed size in bytes.
        limit: usize,
    },

    /// A peer's identity could not be authenticated. Never ignore this one:
    /// it is how a key-substitution attack shows up.
    #[error("identity verification failed: {0}")]
    Unauthenticated(&'static str),

    /// The referenced chat is not known to this device.
    #[error("unknown chat")]
    UnknownChat,
}

impl CoreError {
    pub(crate) fn crypto(context: &'static str, source: impl Into<Source>) -> Self {
        Self::Crypto {
            context,
            source: source.into(),
        }
    }

    pub(crate) fn mls(context: &'static str, source: impl Into<Source>) -> Self {
        Self::Mls {
            context,
            source: source.into(),
        }
    }

    pub(crate) const fn malformed(what: &'static str, reason: &'static str) -> Self {
        Self::Malformed { what, reason }
    }
}

/// Result alias used throughout the core.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;
