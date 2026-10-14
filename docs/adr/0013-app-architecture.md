# 0013. App architecture: one Messenger, a bridge session, keys in the OS vault

- **Status:** Accepted
- **Date:** 2026-10-14

## Context

Phase 4 turns the core into an app people use. The app must:
- stay testable without the native library or a network;
- never lose a message or a key;
- add as few dependencies as possible, each with a permissive license.

The core is sans-IO (ADR-0012), so the app moves the bytes.

## Decision

**One seam: `Messenger`.**
- Screens see only an abstract `Messenger`, a `ChangeNotifier` with a
  stream of changed chats.
- `LiveMessenger` implements it on the Rust core and the network.
- Widget tests use an in-memory fake.
- No state-management framework: Flutter's own `ChangeNotifier` and
  `ListenableBuilder` cover the app.

**The bridge exposes a `Session`, not the core's types.**
- `app/rust/src/api/session.rs` returns plain values to Dart:
  - `HttpCall`, the request to send;
  - `PendingSend`;
  - `InboxEvent`, with the acknowledgement frame to send back;
  - `ChatPreview` and `MessageView`.
- Errors carry stable codes. The app turns them into words
  (`ui/words.dart`), never showing raw errors.
- Timestamps cross as signed 64-bit integers.

**Network: `package:http` and `dart:io` WebSocket.** Both are pure Dart.
- Envelopes are opened strictly one at a time, in mailbox order.
- The outbox is flushed after every send, on every reconnect, and on a
  timer after failures.
- Reconnects back off up to 30 seconds.

**Keys and data: our own vault, sealed by the OS (`os_vault.dart`).**
- The 32-byte master key is stored only sealed, in `key.sealed` next to the
  store. The seal depends on the platform:
  - **Windows:** DPAPI for the signed-in user, called directly through FFI
    (`crypt32.dll`). It carries a Zelamar context string and never shows a
    prompt. The plain key is wiped from native memory after use.
  - **Android:** a hardware-backed AES-256-GCM key in the Android Keystore
    (`MainActivity.kt`, about 60 lines). The sealed form is IV, ciphertext
    and tag. Opening never creates a key; it fails instead.
- The sealed key is written atomically (temporary file, then rename), and
  **before** the store is created. A crash never leaves a store nobody can
  open.
- `flutter_secure_storage` was tried and dropped:
  - its Windows part needs Visual Studio's ATL;
  - its Android default (`resetOnError`) silently erases everything on an
    error.

  The most sensitive part of the app is now short, ours and reviewable.
- Android cloud backup and device transfer are off. Messages never leave
  the device that way, and a restored sealed key could not be opened on
  another phone anyway.
- The encrypted store lives in the app's support directory
  (`path_provider`), one folder per profile.

**Profiles.**
- `--profile=` on the desktop command line (or `ZELAMAR_PROFILE`) keeps
  local accounts apart, so two windows can talk on one machine
  (`app/tool/demo.ps1`).
- The service origin is `--service=` or `ZELAMAR_SERVICE`.

## Consequences

- Three new Dart dependencies: `http`, `path_provider` and `ffi`, all
  BSD-3-Clause and pure Dart or FFI. Flutter plugins need Windows Developer
  Mode on the build machine (enabled 2026-10-14).
- Other platforms (iOS, macOS, Linux) get their sealer when they ship; until
  then the vault refuses to run there rather than store a key in the
  clear.
- Every screen is covered by widget tests against the fake.
  - `integration_test/app_test.dart` runs the real app in CI.
  - `integration_test/live_test.dart` runs two real clients against a
    running service. It is skipped without one.
- Push notifications, multi-device and attachments plug into the same
  `Messenger` seam later.
