# 0003. End-to-end encryption with MLS via OpenMLS

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

- Zelamar promises that nobody but the participants can read messages, not
  even Zelamar's own servers.
- MVP includes 1:1 chats, group chats and multiple linked devices per account.
- We must never invent our own cryptographic protocol. The prototype did, and
  it had no forward secrecy, no sender authentication and silent key
  replacement.
- The CEO decided to keep Zelamar's unique know-how closed source.
  `libsignal`, the reference Signal Protocol implementation, is licensed
  **AGPL-3.0**. Linking it would force the entire client to be open source.

## Decision

Use **Messaging Layer Security (MLS, IETF RFC 9420)** through **OpenMLS**
(Rust, MIT-licensed).

- Every chat is an MLS group: a 1:1 chat is a group of two accounts. One
  mechanism covers 1:1, groups and multi-device.
- Every device is its own MLS member (leaf). An account's long-term Ed25519
  identity key signs its device credentials. Linking a device means an
  existing trusted device signs the new device's key (by QR code or code).
- Devices publish KeyPackages so chats can start while the peer is offline.
- Users can compare safety codes (QR or words). A changed peer key always
  produces a visible warning; keys are never replaced silently.

## Consequences

- Forward secrecy and post-compromise security come from the standard.
- MLS commits (membership changes) need ordering. The delivery service orders
  them online; Drift initially carries only application messages
  (ADR-0004).
- Post-quantum protection: we will evaluate OpenMLS post-quantum ciphersuite
  support in Phase 2 and keep an upgrade path.
- Crypto code stays auditable and can be published even while other parts
  remain closed.
