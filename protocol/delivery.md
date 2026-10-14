# Delivery — version 0

**Status:** draft, normative for protocol version 0. Server:
`server/src/`. Rationale: ADR-0012.

The delivery service moves envelopes (`envelope.md`) into device mailboxes
and hands them to their owner. It never inspects a payload.

## Conventions

- All endpoints live under `https://<host>/v0/`.
- Bytes inside JSON are base64url without padding.
- Integers in binary frames are unsigned big-endian.
- An error is a JSON body `{"error": "<code>"}` with a fitting HTTP status.
  Codes are stable, lowercase ASCII.

## Mailboxes

A mailbox is created when a device registers (`directory.md`). It stores
`SHA-256(secret)` for a 32-byte secret only its device knows.

| Limit | Value |
|---|---|
| Envelopes queued per mailbox | 10 000 (deposits beyond are refused with `mailbox_full`) |
| Time an envelope stays queued | 30 days, then it is dropped |

## Deposit

`POST /v0/deliver` with a binary body, the **delivery frame**:

| Size | Field |
|---|---|
| 2 | magic `ZD` (`0x5A 0x44`) |
| 1 | version `0` |
| 16 | envelope id |
| 1 | recipient count `r`, 1–255 |
| 32·r | recipient mailbox ids, distinct |
| 4 | payload length `n`, at most 1 048 576 |
| n | payload (one MLS message) |

- The service stores, in every recipient mailbox, the envelope
  `ZE | 0 | id | that mailbox | n | payload`.
- A recipient that does not exist is skipped. The response says how many
  were reached: `202 {"delivered": k}`.
- A deposit carries no sender identity. Anonymous tokens (ADR-0012) will
  gate it; until then it is rate-limited by network address.
- An envelope id already queued in a mailbox is not queued twice
  (idempotent retries).

Errors: `400 malformed`, `413 too_large`, `429 rate_limited`.

## Reading a mailbox

`GET /v0/mailboxes/{mailbox}` with `Upgrade: websocket` and
`Authorization: Bearer <secret, base64url>`.

- `401 unauthorized` if the secret does not match, `404 unknown_mailbox` if
  there is no such mailbox.
- On connect, the service sends every queued envelope, oldest first, one
  binary WebSocket message each (the envelope encoding). Envelopes
  deposited while connected follow immediately.
- The device acknowledges with a binary message: `0x01` followed by one or
  more 16-byte envelope ids. Acknowledged envelopes are deleted for good.
- Unacknowledged envelopes are sent again on the next connection. Devices
  drop duplicates by envelope id.
- One device may hold several connections. Each new envelope goes to all
  of them.

## Ordered commits

See ADR-0012. Specified together with the core's pending-commit lifecycle.
