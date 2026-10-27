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
- The header `Zelamar-Wake: call` marks the envelope urgent: a call's
  offer (`calls.md`). It changes only how a device that is away is woken.

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

## Push wake-ups

A device that cannot stay connected, such as Android after the app is
closed, may leave a push token with its mailbox (ADR-0017).

`PUT /v0/mailboxes/{mailbox}/push`, with `Authorization: Bearer <secret>`
and the JSON body `{"platform": "fcm", "token": "<FCM registration token>"}`.
The token is 1–4096 characters. `DELETE` on the same path forgets it.

| Answer | When |
|---|---|
| `200 {}` | kept, or forgotten |
| `401 unauthorized` | the mailbox does not exist, or the secret is wrong |
| `400 malformed` | another platform, or a bad token |

Rules:
- When an envelope is queued and no connection is open, the mailbox owes a
  wake-up. It sends one at most every 30 seconds, and only if the envelope
  is still unread when the wake-up goes out.
- A wake-up carries the same fixed words every time ("Zelamar", "New
  message"), never anything about the message.
- An urgent envelope owes a **call wake-up** instead: at most one every 5
  seconds per mailbox, sent at once. It is a high-priority FCM data
  message, `{"wake": "call"}`, that lives 45 seconds and shows nothing by
  itself. The app wakes, fetches its mail and rings only for a real call
  from someone in an accepted chat, so a forged wake-up rings nothing.
- A token that Firebase reports as unregistered is forgotten.

## Client rules

Implemented without I/O in `core/zelamar-core/src/service.rs`. The app only
moves the bytes.

1. **Sending.** Each outbox item goes out as one delivery frame. The item
   is confirmed (removed from the outbox) once the service answers `202`.
   On `429` or `5xx` the client retries later, in order.
2. **Receiving.** The client opens each envelope from its mailbox: a message,
   a chat update, or a welcome. A welcome joins the chat, which the app
   shows as a chat request naming the inviter.
3. **Acknowledging.** Every envelope is acknowledged, delivered or refused
   for good, so a bad envelope never blocks the queue. The one exception is
   a transient failure (the device's own storage), which leaves the
   envelope for the next attempt.
4. **Starting a chat.** The client starts a chat from a directory lookup
   with all of the contact's devices. Every key package must be certified by
   the looked-up account; otherwise nothing is created.

## Ordered commits

MLS breaks if two members commit in the same epoch and each applies their
own. Every commit that others must apply therefore goes through its chat's
sequencer: `POST /v0/groups/{group}/commits`, where `{group}` is the MLS
group id (16 bytes, base64url). The body is a **commit frame**:

| Size | Field |
|---|---|
| 2 | magic `ZC` (`0x5A 0x43`) |
| 1 | version `0` |
| 8 | the epoch the commit was made in |
| … | a delivery frame with the commit for the current members (it may have no recipients) |
| … | optionally, a delivery frame with the welcome for added members |

The sequencer keeps only the chat's current epoch and the last winning
commit's id. It does not keep members or content.

| Request | Answer |
|---|---|
| The first commit for the current epoch, or the first ever for this chat | `202 {"epoch": <new>, "delivered": k, "welcomed": w}`; the commit, then the welcome, go out |
| The winning commit again (a retry) | `202` again, delivered once (envelopes are idempotent) |
| Any other commit | `409 {"error": "epoch_mismatch", "epoch": <current>}` |

A device applies its own commit only after the `202`. After a `409`, it
drops the commit, applies the winning one from its mailbox, and retries if
it still needs to.

Client rules:

- A commit in a chat with no one else in it is applied at once and never
  sent; its welcome goes out as an ordinary delivery.
- While a device's own commit waits for an answer, messages it sends still
  use the current epoch. Receivers keep the last 4 epochs' keys, so these
  messages stay readable after the commit lands.
- If the winning commit arrives before the answer, the device resolves its
  own from the epoch alone: a commit for the same epoch means it lost (its
  commit is dropped, together with its queued copy); one for the next epoch
  means it won and the answer was lost (its commit is applied first).
