# Message content — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/content.rs`.

The plaintext of every MLS application message is one CBOR map:

| Key | Type | Meaning |
|---|---|---|
| `v` | uint | content version, `0` |
| `i` | bytes(16) | message id chosen by the author |
| `t` | uint | sent at: UTC milliseconds since the Unix epoch, author's clock |
| `k` | text | body kind, at most 32 bytes |
| `c` | map | the kind's fields |

## Kinds in version 0

| `k` | Fields of `c` | Meaning |
|---|---|---|
| `text` | `text`: string (≤ 32 768 bytes) | A message |
| `reply` | `to`: bytes(16), `text`: string | A message answering `to` |
| `edit` | `target`: bytes(16), `text`: string | New text for one of the author's own messages |
| `react` | `target`: bytes(16), `reaction`: string (1–32 bytes) or null | Sets, or with null clears, the author's reaction |
| `delete` | `target`: bytes(16) | Deletes one of the author's own messages for everyone |
| `receipt` | `status`: `"delivered"` or `"read"`, `messages`: array of bytes(16), 1–256 items | Acknowledges messages |
| `profile` | `username`: string, a normalized username (`directory.md`) | Who the author is; never shown as a message |

Receivers MUST check, against their own history, that an `edit` or
`delete` targets a message by the same author (same account); otherwise
they ignore it. A `receipt` only ever updates the receiver's own messages.
A message id is unique within a chat: a `text`, `reply` or unknown kind
whose `i` is already in the receiver's history MUST be rejected.

A device sends its `profile` once per chat:
- when it starts the chat;
- in a chat it was invited to, right before its own first message there.

So someone who declines a chat request reveals nothing. The username in a
profile is only a claim. Receivers show it as confirmed only after the
directory (`GET /v0/accounts/{username}/key`) names the same account.

## Rules

1. The encoded content is at most 65 536 bytes. Larger content is rejected
   before decoding.
2. A receiver MUST reject a version above the one it implements, and report
   it instead of misreading it.
3. An unknown `k` is surfaced to the user as "update Zelamar to see this
   message", never as an error that breaks the chat. It is never re-sent.
4. Unknown keys inside `c` are ignored, which leaves room for evolution.
   Fields that do not match the kind make the message malformed.
5. Display time is always `t` converted to the viewer's time zone, never
   the receive time. (The prototype got this wrong.)
