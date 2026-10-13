# 0002. Technology stack: Flutter + Rust core + TypeScript on Cloudflare

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

- MVP platforms are Android and Windows; iOS and macOS follow. One small team
  (one CEO, one AI CTO) must ship all of them.
- The CEO is a UI perfectionist: the interface must look identical and premium
  on every platform.
- Security-critical code (keys, encryption, storage) must be written once and
  shared, not reimplemented per platform.
- Future offline delivery (Drift) needs a portable networking/storage core.
- The infrastructure budget is $0 until there is a real product.
- The previous prototype (Electron + Node) suffered from weak typing and
  duplicated logic between processes.

## Decision

| Layer | Choice |
|---|---|
| UI | **Flutter** (Dart). It paints every pixel itself, so the UI is identical on Android, Windows, iOS and macOS. |
| Core | **Rust** crate `zelamar-core`: identity, MLS, storage, send queue, transports. Pure Rust, no FFI concerns. |
| Bridge | **flutter_rust_bridge** v2 in a thin `zelamar_bridge` crate (`app/rust`). Business logic never lives in the bridge. |
| Server | **TypeScript** (strict) on **Cloudflare Workers + Durable Objects** (SQLite-backed, available on the free plan). Files in **R2**. |
| Push | Firebase Cloud Messaging (Android), carrying only an empty "wake up". |
| CI | GitHub Actions. |

The server is a separate language on purpose. It is a dumb, zero-knowledge
delivery service whose contract is the envelope format in `protocol/`, not
shared Rust types.

## Consequences

- Three languages (Dart, Rust, TypeScript), each the best tool for its layer.
  Strict tooling per language keeps quality uniform (ADR-0006).
- The free tiers fit a closed beta comfortably. The server must stay portable:
  no logic may depend on Cloudflare-only behavior without an ADR.
- Rust compile times affect app build times; CI caches mitigate this.
- Rejected: Electron (heavy, prototype issues), React Native (UI not
  pixel-identical across platforms), per-platform native UIs (team too small),
  Kotlin Multiplatform (weaker Windows desktop story).
