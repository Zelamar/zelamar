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
| `delivery.md` | Delivery service API: mailboxes, sealed-sender tokens, acknowledgements, ordering of MLS commits |
| [`identity.md`](identity.md) ✅ | Account and device keys, device certificates (linking and safety codes to come) |
| [`content.md`](content.md) ✅ | What travels inside an MLS application message |
| `directory.md` | `@username` registry, KeyPackage publication and retrieval |
| `attachments.md` | Client-side encrypted files: upload, reference, expiry |

## Invariants (already binding)

1. The server never receives plaintext, sender identity, contact lists or
   group membership.
2. Envelopes are transport-agnostic and idempotent (ADR-0004).
3. Timestamps are sender-side UTC milliseconds.
4. Every format carries a version; unknown versions are rejected explicitly,
   never guessed.
