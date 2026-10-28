# Architecture Decision Records

Every significant technical decision is recorded here: the context, the
decision, and its consequences. ADRs are immutable once accepted; a changed
decision gets a new ADR that supersedes the old one.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-technology-stack.md) | Technology stack: Flutter + Rust core + TypeScript on Cloudflare | Accepted |
| [0003](0003-mls-via-openmls.md) | End-to-end encryption with MLS via OpenMLS | Accepted |
| [0004](0004-transport-agnostic-envelopes.md) | Transport-agnostic envelopes (Drift-ready) | Accepted |
| [0005](0005-dependency-license-policy.md) | Dependency and license policy | Accepted |
| [0006](0006-quality-gates-and-workflow.md) | Quality gates and workflow on GitHub Free | Accepted |
| [0007](0007-rust-in-flutter-via-native-assets.md) | Build Rust into Flutter via Native Assets | Accepted |
| [0008](0008-design-system-in-code.md) | Design system lives in code | Accepted |
| [0009](0009-core-identity-ciphersuite-and-encodings.md) | Core: device certificates, post-quantum ciphersuite, encodings | Accepted |
| [0010](0010-encrypted-device-store.md) | Encrypted device store: SQLite + per-row AEAD over MLS state diffs | Accepted |
| [0011](0011-addressing-outbox-and-history.md) | Device addressing in the MLS tree, durable outbox, encrypted history | Accepted |
| [0012](0012-delivery-service.md) | Delivery service: Durable Objects, sans-IO core, anonymous tokens, ordered commits | Accepted |
| [0013](0013-app-architecture.md) | App architecture: one Messenger, a bridge session, keys in the OS vault | Accepted |
| [0014](0014-linked-devices.md) | Linked devices: a typed code, a certificate, chats that heal themselves | Accepted |
| [0015](0015-group-chats.md) | Group chats: one MLS group, names in messages, people leave by themselves | Accepted |
| [0016](0016-attachments.md) | Attachments: encrypted chunks in Durable Objects, keys in the message | Accepted |
| [0017](0017-push-wake-ups.md) | Push: an empty wake-up through Firebase, sent only when a device is away | Accepted |
| [0018](0018-account-recovery.md) | Account recovery: a key only the user holds, and chats that heal across accounts | Accepted |
| [0019](0019-calls.md) | Calls: WebRTC between two devices, set up inside the encrypted chat | Accepted |
| [0020](0020-drift-local-network.md) | Drift D1: messages over the local network when the internet is gone | Accepted |

New ADRs copy [`template.md`](template.md) and take the next number.
