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
| `name` | `name`: string, 1–64 characters, no surrounding spaces, no control characters | Names the chat (a group); shown as a line |
| `leave` | none | The author leaves the chat; shown as a line |
| `file` | as in `attachments.md` | A photo, document or voice message |

Receivers MUST check, against their own history, that an `edit` or
`delete` targets a message by the same author (same account); otherwise
they ignore it. A `receipt` only ever updates the receiver's own messages.
A message id is unique within a chat: a `text`, `reply` or unknown kind
whose `i` is already in the receiver's history MUST be rejected.

A device sends its `profile` once per chat:
- when it starts the chat;
- in a chat it was invited to, right before its own first message there.

So someone who declines a chat request reveals nothing.

A device sends receipts only in chats it has introduced itself in, that is,
chats the user started or wrote in:
- a `delivered` receipt as each `text`, `reply` or unknown kind arrives from
  someone else;
- a `read` receipt (the newest 256 at most) when the user sees the chat.

Until the user writes, a stranger learns nothing, not even that a message
arrived.

Groups (ADR-0015):
- A chat's name is the `name` with the greatest `(t, i)`. A device that adds
  people sends the current name again once its change is accepted, so the
  newcomers learn it. A `name` equal to the current one is not shown again.
- After a `leave` from an account, the remaining member with the lowest leaf
  index takes all of that account's devices out of the chat, in one
  sequenced commit. Adding that account again cancels it.
- A `leave` from one's own account makes every own device leave too.
- Someone leaving a chat they never wrote in (a request) sends nothing.

The username in a profile is only a claim. Receivers check it with the
directory (`GET /v0/accounts/{username}/key`) as soon as it arrives:
- if the directory names the same account, the name is confirmed;
- if it names another account, or no one holds the name, the claim is
  dropped, and that account's later claims of the same name are ignored.

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
