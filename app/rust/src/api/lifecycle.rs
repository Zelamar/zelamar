//! Bridge lifecycle hooks.

/// Runs once when Dart calls `RustLib.init()`.
#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}
