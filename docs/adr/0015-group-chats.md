# 0015. Group chats: one MLS group, names in messages, people leave by themselves

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

Version 1.0 needs group chats for friends: a name, a few to a few dozen
people, adding people later, and leaving. The existing pieces cover most of
this:
- every chat is already an MLS group (ADR-0003);
- every change is already sequenced (ADR-0012);
- linked devices already follow their account into every chat (ADR-0014).

## Decision

**A group is a chat with a name, or with more than one other account.**
- To create one, the user picks a name and people found by exact
  `@username`. One commit adds every device of every person, plus the
  creator's other devices. The name follows as a message.
- At most 50 accounts per chat in v1.
- No server change: a group is a chat.

**The name travels as a message (`name`, `protocol/content.md`).**
- The newest one by `(sent time, message id)` wins. It is shown as a line:
  "@ada named the group “Family”".
- New members see no history. So a device whose add is accepted sends the
  current name again. Those who already know it see nothing new.
- MLS group context extensions would need a commit per rename and an
  admin. A message is simpler and just as private.

**Anyone in the chat adds people.** The commit goes through the sequencer,
with a welcome for the newcomers.

**People leave by themselves.** MLS does not let a member commit its own
removal.
- The leaving device sends `leave`, erases the chat and stops reading it.
- Its other devices see their own account's `leave` and erase the chat too.
- One device takes the leaver's devices out: the remaining member with the
  lowest leaf index. It is the same choice everywhere, so no one races it.
  If that member is offline, the leaver's devices stay members until it
  returns. They already erased the chat, so they read nothing, but the
  server keeps delivering to them for a while.

**No one removes anyone else in v1.** In a commit, any member can remove
any leaf. A client that refused such commits would fork the chat, so honest
clients accept every sequenced commit and simply never offer the action.
Roles (an admin who may remove people) are a later decision. They need
group context extensions and receivers that check commits before the
sequencer sees them.

**Privacy stays as in 1:1 chats.** A group someone else started is a
request until the user accepts it. Until then nothing is sent: no profile,
no receipts, and no `leave` when the user declines.

## Consequences

- Groups need no server change, and healing (ADR-0014) works in them
  unchanged.
- A modified client could still remove people from a group. Everyone would
  see the change, and anyone can add them back. That is acceptable for a
  closed beta of friends.
- Receipts in groups count per account; the bubble shows the best status
  any member reached. "Read by 3" and the full list are later work.
- Events are recorded in history as `Named` and `Left` records. Who was
  added or removed by a commit is not shown as a line yet.
