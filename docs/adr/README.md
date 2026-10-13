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

New ADRs copy [`template.md`](template.md) and take the next number.
