# 0007. Build Rust into Flutter via Native Assets

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

flutter_rust_bridge 2.13 offers two ways to compile and bundle the Rust bridge
crate with the app:

- **cargokit** (default): a vendored build tool plus a generated platform
  plugin (`rust_builder/`). Battle-tested, but it adds thousands of lines of
  third-party build scripts to the repository.
- **Native Assets**: Flutter's official build-hook mechanism (`hook/build.dart`
  using `native_toolchain_rust`). It is newer, but it is the platform's own
  direction and adds a single small file.

## Decision

Use **Native Assets**. The bridge crate lives in `app/rust`. It is a member of
the root Cargo workspace and depends on `core/zelamar-core` by path.

`native_toolchain_rust` requires an exact toolchain version, so
`app/rust/rust-toolchain.toml` pins the same channel as the root
`rust-toolchain.toml` and lists every compilation target we ship. Both files
must always change together.

## Consequences

- Clean repository and the officially supported build path.
- Verified on 2026-10-13: Windows debug build plus an integration test calling
  the real core. Android is verified by the CI emulator job.
- If Native Assets regresses for a platform, falling back to cargokit is a
  mechanical change (`flutter_rust_bridge_codegen integrate
  --integration-backend cargokit`) recorded in a superseding ADR.
