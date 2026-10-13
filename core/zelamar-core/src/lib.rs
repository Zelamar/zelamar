//! Zelamar Core — the platform-independent heart of Zelamar.
//!
//! Every Zelamar client (Android, Windows, later iOS and macOS) runs this same
//! crate. Identity, end-to-end encryption, encrypted storage and message
//! transports live here, so security-critical logic is written exactly once.
//!
//! The crate is intentionally free of any FFI or UI concerns: the Flutter app
//! talks to it through a thin bridge crate (`app/rust`).
//!
//! Modules:
//! - [`identity`]: account keys and device certificates;
//! - [`device`]: a device as a member of MLS chats;
//! - [`content`]: what travels inside a message;
//! - [`envelope`]: the transport-agnostic wire unit;
//! - [`history`]: the local history of each chat;
//! - [`store`]: encrypted persistence of a device.

pub mod content;
pub mod device;
pub mod envelope;
pub mod error;
pub mod history;
pub mod identity;
pub mod store;

pub use error::{CoreError, Result};

/// Semantic version of this core build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the Zelamar wire protocol (envelope format) spoken by this core.
///
/// `0` means "pre-release": the format may still change without migration.
pub const PROTOCOL_VERSION: u16 = 0;

/// Static facts about the running core, shown in diagnostics and "About".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreInfo {
    /// Semantic version of the core, see [`VERSION`].
    pub version: &'static str,
    /// Wire protocol version, see [`PROTOCOL_VERSION`].
    pub protocol_version: u16,
}

/// Returns static information about this core build.
#[must_use]
pub const fn info() -> CoreInfo {
    CoreInfo {
        version: VERSION,
        protocol_version: PROTOCOL_VERSION,
    }
}

/// Builds the greeting shown by the Phase 0 "Hello, Zelamar" screen.
///
/// Surrounding whitespace is ignored; an empty name greets Zelamar itself.
#[must_use]
pub fn greet(name: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { "Zelamar" } else { name };
    format!("Hello, {name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_reports_crate_and_protocol_versions() {
        let info = info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn greet_uses_trimmed_name() {
        assert_eq!(greet("  Ada  "), "Hello, Ada");
    }

    #[test]
    fn greet_falls_back_to_zelamar_for_blank_names() {
        assert_eq!(greet(""), "Hello, Zelamar");
        assert_eq!(greet(" \t\n"), "Hello, Zelamar");
    }

    #[test]
    fn greet_preserves_non_ascii_names() {
        assert_eq!(greet("Zəlamar"), "Hello, Zəlamar");
    }
}
