//! Zelamar bridge: the only place where Rust meets Dart.
//!
//! Business logic never lives here; it belongs in `zelamar-core`. This crate
//! only adapts core types to shapes `flutter_rust_bridge` can carry.

pub mod api;
mod frb_generated;
