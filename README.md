# Zelamar

**Private by design. Beautiful by obsession.**

Zelamar is a premium, end-to-end encrypted messenger: maximum privacy with no
loss of comfort, an account in seconds (`@username`, no phone, no e-mail), and
a long-term goal of working without internet (**Drift**).

> Proprietary and confidential. © 2026 Zelamar. All rights reserved.

## Where to start

| Document | What it answers |
|---|---|
| [`ROADMAP.md`](ROADMAP.md) | What we build, in which order, and the status (source of truth, in Russian for the CEO) |
| [`docs/adr/`](docs/adr/) | Why each technical decision was made |
| [`protocol/`](protocol/) | The wire format: envelopes and how clients talk |

## Repository layout

```
app/        Flutter app (Android, Windows; later iOS, macOS)
  rust/     zelamar_bridge — thin FFI bridge to the core (flutter_rust_bridge)
  hook/     Native Assets build hook that compiles the bridge
core/       zelamar-core — identity, encryption (MLS), storage, transports (Rust)
server/     Delivery service on Cloudflare Workers (TypeScript)
protocol/   Envelope and wire-format specification
docs/adr/   Architecture Decision Records
scripts/    Developer scripts (full local check)
.githooks/  Local git hooks (fast formatting gate)
```

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Flutter | 3.47.7 (stable) | App |
| Rust | 1.99.0 (pinned by `rust-toolchain.toml`, installed automatically) | Core and bridge |
| flutter_rust_bridge_codegen | 2.13.0 (`cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked`) | Regenerating the bridge |
| cargo-deny | 0.20+ (`cargo install cargo-deny --locked`) | License and advisory checks |
| Node.js | 22+ | Server |
| Visual Studio Build Tools 2022 | "Desktop development with C++" | Windows builds |

One-time setup after cloning:

```sh
git config core.hooksPath .githooks
(cd server && npm ci)
(cd app && flutter pub get)
```

## Everyday commands

```sh
scripts/check.sh                                   # everything CI checks, locally
(cd app && flutter run -d windows)                 # run the app
(cd app && flutter_rust_bridge_codegen generate)   # after changing app/rust/src/api
(cd server && npm run dev)                         # local delivery service
```

## Quality gates

Nothing reaches `main` without a pull request and green CI: strict lints with
warnings as errors in all three languages, tests, license policy, a real Windows
build and an Android emulator run. See
[ADR-0006](docs/adr/0006-quality-gates-and-workflow.md).
