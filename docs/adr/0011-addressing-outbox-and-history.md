# 0011. Device addressing in the MLS tree, durable outbox, encrypted history

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

To deliver a message, a sender must know where each recipient device
receives mail. A message written offline must never be lost: the prototype
showed ✓ and dropped it. The app needs a local history with edits,
reactions, deletions and receipts. All of it must be crash-consistent with
the MLS state (ADR-0010) and must not leak to the server.

## Decision

**Addressing: mailbox ids live in the MLS tree.**
- Every device draws a random 32-byte `MailboxId` and publishes it as a
  leaf-node extension (type `0xF5A1`, private-use range) in its
  `KeyPackage` and in the leaf of every chat it creates. Its capabilities
  advertise that extension type.
- A sender reads every other member's mailbox from the group's ratchet
  tree. There is no directory lookup, so the server never learns who looks
  up whom. Drift can address devices without any server.
- A member without a well-formed mailbox is refused, like a member without
  a valid certificate: an unreachable member is a broken member. This is
  checked for every leaf that enters a chat: the invited key package, every
  member on join, and in every received commit the added members, updated
  leaves and the committer's new leaf.
- Rotation later happens through an MLS update of the leaf.

**Outgoing: one ciphertext, many mailboxes.**
`Device::send` produces an `Outgoing { id, recipients, payload }`: one MLS
ciphertext plus the mailbox ids of all other members. The delivery service
fans it out (Phase 3). The server learns the recipients of a message, as in
Signal, but never its sender (sealed sender).

**Outbox: durable until confirmed.**
- Every `Outgoing` (messages, commits and welcomes) is written to an
  encrypted `outbox` table in the **same transaction** as the MLS state
  change that produced it. Either the ratchet advanced and the ciphertext is
  queued, or neither happened.
- A transport takes items from `Device::outbox()` and calls
  `confirm_sent(id)` after the delivery service acknowledges. Items are
  retried in order until then. Confirming twice is harmless.
- An `Outgoing` with no recipients (a chat with no one else in it) is not
  queued; a message sent there is `Sent` at once.

**History: encrypted, per chat, in arrival order.**
- A `messages` table holds the following:

  | Column | Content |
  |---|---|
  | `chat` | HMAC of the chat id |
  | `seq` | local arrival order |
  | `slot` | HMAC of chat id and message id, unique |
  | `sealed` | a `StoredMessage`: id, author, direction, author's timestamp, text, edit time, deleted flag, reactions per account, delivered-to and read-by sets, send status |

- Applying content:
  - **text and reply** insert;
  - **edit and delete** apply only to the sender's own messages (same
    account). A deleted message keeps no text;
  - **react** sets or clears the sender's reaction;
  - **receipt** updates only this device's own messages;
  - a message id reused by another author is rejected;
  - an unknown kind is stored as such, so the app can say "update Zelamar".
- History writes join the operation's transaction (`Batch`), so history,
  outbox and MLS state never disagree.

## Consequences

- Visible on disk: the number of rows per chat (seq) and the outbox
  length. Not visible: any id, name, content or timestamp.
- **What the delivery service can learn.** It sees which mailboxes each
  message goes to. Key packages are published under an `@username` and
  carry the device's mailbox, so a server that inspects them can link a
  mailbox to a username. It could therefore infer who shares a chat with
  whom and when they write. It cannot learn content or the sender, and it
  never stores contact lists or memberships. This is the same class of
  routing metadata Signal's server sees. Hiding it needs per-chat mailboxes
  rotated after joining plus unlinkable connections. That is a later ADR,
  not v1.
- The transport layer is simple: drain the outbox, confirm, feed received
  bytes to `receive`.
- Breaking changes for protocol version 0 (pre-release): key packages
  without a mailbox extension are refused.
