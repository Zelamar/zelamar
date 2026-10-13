# 0005. Dependency and license policy

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

Zelamar opens its cryptography but keeps its know-how closed (CEO decision).
A single copyleft dependency (GPL, LGPL, AGPL) could legally force us to open
code we intend to keep closed. Supply-chain attacks are also a real risk for a
security product.

## Decision

- Only permissive licenses are allowed. The allow-list lives in `deny.toml`:
  MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 (file-level copyleft only), and
  similar. Copyleft licenses are absent and therefore rejected.
- `cargo-deny` runs in CI on every Rust change. It also rejects crates with
  known vulnerabilities, yanked versions, unknown registries and git sources.
- Dart and npm dependencies are reviewed manually when added, against the same
  allow-list, until automated checks exist for them.
- Every new dependency must be justified in its PR: what it does and why we
  should not write it ourselves.
- Bridge versions are pinned exactly (`flutter_rust_bridge = "=2.13.0"` on
  both sides).

## Consequences

- License mistakes are caught before merge, not by lawyers later.
- Some useful libraries will be off-limits. That is the price of the open/closed
  split.
- TODO (Phase 2): automate license checks for Dart and npm dependencies too.
