//! Diagnostics about the running core: used by the About screen and by the
//! Phase 0 bridge smoke test.

/// Static facts about the running Zelamar core.
pub struct CoreInfo {
    /// Semantic version of the core.
    pub version: String,
    /// Wire protocol (envelope format) version spoken by the core.
    pub protocol_version: u16,
}

/// Returns static information about the running core.
#[flutter_rust_bridge::frb(sync)]
#[must_use]
pub fn core_info() -> CoreInfo {
    let info = zelamar_core::info();
    CoreInfo {
        version: info.version.to_owned(),
        protocol_version: info.protocol_version,
    }
}

/// Greets `name` through the core. Proves that Dart reaches Rust and back.
#[flutter_rust_bridge::frb(sync)]
#[must_use]
pub fn greet(name: String) -> String {
    zelamar_core::greet(&name)
}
