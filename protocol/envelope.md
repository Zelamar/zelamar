# Envelope — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/envelope.rs`.

An envelope is the only thing any transport carries: the delivery service
today, Drift transports later (ADR-0004). It reveals nothing about the sender,
the chat or the time; those live encrypted inside the payload.

## Layout

All integers are unsigned big-endian.

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 2 | magic | ASCII `ZE` (`0x5A 0x45`) |
| 2 | 1 | version | `0` |
| 3 | 16 | envelope id | random; receivers drop duplicates |
| 19 | 32 | mailbox id | opaque address of the recipient device's mailbox |
| 51 | 4 | payload length | `n`, at most 1 048 576 |
| 55 | n | payload | one TLS-encoded MLS message (`MLSMessage`) |

## Rules

1. A receiver MUST reject:
   - a wrong magic;
   - an unknown version (it reports it, never guesses);
   - a declared length above the limit;
   - truncated data;
   - trailing bytes.
2. A relay (delivery service, Drift peer) MUST NOT inspect or modify the
   payload. It routes by mailbox id only.
3. Envelope ids are 16 bytes from a CSPRNG. Duplicates by id are idempotent.
4. The mailbox id is not derived from any identity key, so it can rotate.
