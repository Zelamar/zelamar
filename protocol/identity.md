# Identity — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/identity.rs` and `device.rs`.

## Keys

- **Account key:** Ed25519. The account id is the 32-byte public key.
- **Device signature key:** the MLS signature key of one device (Ed25519,
  per the ciphersuite in `device.rs`).

## Device certificate

The account key signs the following payload:

```text
"zelamar/device-certificate/v0" (ASCII, 29 bytes)
|| account id (32)
|| device id (16)
|| device key length (u16 BE) || device key
```

Encoding, carried as the MLS `BasicCredential` identity:

| Size | Field |
|---|---|
| 1 | version `0` |
| 32 | account id |
| 16 | device id |
| 2 + k | device key, u16 BE length prefix (k = 32) |
| 2 + s | signature, u16 BE length prefix (s = 64) |

## Verification

A member is accepted only if **all** of the following hold:
1. The credential is a `BasicCredential` whose identity decodes as a
   version 0 certificate, with no trailing bytes.
2. The Ed25519 signature verifies under the certificate's account id.
3. The certified device key equals the member's MLS leaf signature key.

These checks run on invite (the newcomer's `KeyPackage`), on join (every
member in the Welcome) and on every incoming commit (each added member).
A failure aborts the operation. It is never downgraded to a warning.

## Safety code

Two people confirm that no one sits between them by comparing a 60-digit
code, shown as twelve groups of five digits (QR comes later).

For each of the two account ids, the fingerprint is:

```text
d0 = SHA-512("zelamar/safety-code/v0" || account id)
d(n+1) = SHA-512(d(n) || account id), 5200 rounds
```

Each fingerprint gives six groups: bytes `5k..5k+5` of the final digest
(k = 0..5) are read as a big-endian integer modulo 100 000. The code lists
the six groups of the lexicographically smaller account id first, then the
six groups of the other, so both sides compute the same code.
