# 0009. Core: device certificates, post-quantum ciphersuite, encodings

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

Phase 2 builds the security-critical core. ADR-0003 chose MLS via OpenMLS;
this ADR records the concrete choices of the first core slice: how
identities are bound to MLS members, which ciphersuite, and how content and
envelopes are encoded. OpenMLS 0.9.1 (October 2026) is the current release.

## Decision

**Identity: device certificates.**
- An account owns a long-term Ed25519 key, its identity (`AccountId` is the
  public key).
- Each device has its own MLS signature key. The account key signs a
  `DeviceCertificate` (context `zelamar/device-certificate/v0`, account,
  device id, device key) that travels as the MLS `BasicCredential` identity.
- Every member is verified when it enters a chat (invite, join, incoming
  commits): the certificate must verify **and** certify exactly the signature
  key the member uses. Tests cover a borrowed certificate and a re-labelled
  one; both are refused.

**Ciphersuite: hybrid post-quantum.**
`MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519` (OpenMLS feature
`draft-ietf-mls-pq-ciphersuites`). It protects against "harvest now, decrypt
later": confidentiality holds unless both ML-KEM-768 and X25519 break.
Authentication stays Ed25519 for now. Zelamar is the only implementation that
must interoperate with itself, so a change in the IETF draft means new chats
move to the final codepoint. The ML-KEM implementation is RustCrypto `ml-kem`
with `libcrux-sha3`, and passes `cargo-deny`.

**Wire policy.** Pure ciphertext (handshake messages encrypted too), ratchet
tree carried in the Welcome, padding to 64-byte multiples.

**Encodings.**
- Content inside MLS: CBOR (`ciborium`) with short field names, a version
  field, and hard size limits (64 KiB encoded, 32 KiB text). CBOR is
  self-describing, so future fields do not break old clients.
- Envelope and certificate: tiny fixed big-endian layouts with a magic, a
  version and strict parsing (no trailing bytes). Easy to parse in the
  TypeScript delivery service without a schema compiler.
- All decoders are fuzzed with property tests (`proptest`): arbitrary input
  never panics.

**Errors.** One typed `CoreError`. Library errors are kept as `source` and
never leak plaintext.

## Consequences

- Key substitution by a server or a malicious member is detectable by
  design. Human-verifiable safety codes build on the same account key.
- Post-quantum key packages are larger (an ML-KEM-768 public key is
  1184 bytes), which is acceptable.
- Slice 1 keeps MLS state in memory. Persistent, encrypted storage
  (SQLCipher via `openmls_sqlite_storage`) is the next slice; until then a
  restart loses chats.
- Standalone proposals are rejected. Zelamar issues commits directly.
