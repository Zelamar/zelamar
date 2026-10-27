# Zelamar protocol

This directory will hold the normative specification of how Zelamar clients
talk to each other and to the delivery service. The specification is the
contract between the Rust core and the TypeScript server; neither side may
diverge from it.

**Status:** protocol version `0` (pre-release, may change without
migration). Drafted in Phase 2 together with the core implementation.

## Planned documents

| Document | Content |
|---|---|
| [`envelope.md`](envelope.md) ✅ | Envelope layout: opaque outer header (mailbox id, envelope id) plus inner MLS message |
| [`delivery.md`](delivery.md) ✅ | Delivery service API: deposit, mailboxes over WebSocket, acknowledgements (tokens and commit ordering to come) |
| [`identity.md`](identity.md) ✅ | Account and device keys, device certificates, safety codes |
| [`addressing.md`](addressing.md) ✅ | Mailbox ids in the MLS tree, fan-out of one ciphertext to many mailboxes |
| [`content.md`](content.md) ✅ | What travels inside an MLS application message |
| [`directory.md`](directory.md) ✅ | `@username` registry, signed device requests, KeyPackage publication and retrieval |
| [`linking.md`](linking.md) ✅ | Linking a new device with a typed code; chats that follow the account |
| [`attachments.md`](attachments.md) ✅ | Client-side encrypted files: sealed chunks, the `file` message, the blob store, expiry |
| [`recovery.md`](recovery.md) ✅ | A recovery key, a sealed kit at the service, chats that heal across accounts |
| [`calls.md`](calls.md) ✅ | Calls: WebRTC set up inside the chat, call servers, call records |

## Test vectors

[`vectors/v0.json`](vectors/v0.json) holds byte-exact examples written by the
core (`ZELAMAR_WRITE_VECTORS=1 cargo test -p zelamar-core vectors`):
- username rules;
- a username claim;
- a signed device request;
- a delivery frame, an envelope and an acknowledgement.

The core rebuilds them and the server's tests run them through its
production checks. If the two sides disagree by one byte, CI fails.

## Invariants (already binding)

1. The server never receives plaintext or the sender's identity, and never
   stores contact lists or group membership. It necessarily sees which
   mailboxes a message is delivered to; ADR-0011 states exactly what that
   reveals.
2. Envelopes are transport-agnostic and idempotent (ADR-0004).
3. Timestamps are sender-side UTC milliseconds.
4. Every format carries a version; unknown versions are rejected explicitly,
   never guessed.
