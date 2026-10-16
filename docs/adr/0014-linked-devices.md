# 0014. Linked devices: a typed code, a certificate, chats that heal themselves

- **Status:** Accepted
- **Date:** 2026-10-15

## Context

People use Zelamar on a computer and a phone, and both must be first-class:
every chat, live, end-to-end encrypted. Constraints:

- No phone number or email to fall back on. The account is its key, which
  lives on the device that created it (ADR-0009).
- The server must learn nothing it could use to join an account, even if it
  is malicious.
- The CEO tests without a phone camera, so linking cannot depend on QR codes.
- Commits are sequenced (ADR-0012), so devices may add members concurrently.

## Decision

**The device that holds the account key links new devices with a typed
code.**
- On the existing device: *You → Link a device* shows a code like invites:
  `xxxxx-xxxxx-xxxxx-xxxxx`. It is 100 random bits, used once, valid for 10
  minutes.
- On the new device: *I already use Zelamar* → type the code.
- Both derive a rendezvous slot and a key from the code with HKDF-SHA256.
  The server sees only the slot, never the code or the key.
- Both messages go through a short-lived slot on the server and are sealed
  with ChaCha20-Poly1305:
  1. The **request** (new → existing) carries the new device's id and MLS
     signature key.
  2. The **answer** (existing → new) carries:
     - a device certificate signed by the account key;
     - the username and the contacts;
     - the chats the account has already spoken in.
- Both screens show the same 6-digit **match code**, derived from the key
  and the request. If someone saw the code and raced the real device, the
  numbers differ and the user cancels.
- An offline guess of 100 bits is out of reach, so a plain key derivation
  suffices; a PAKE is not needed.

**A linked device never holds the account key.** It cannot link others or
claim a name. Losing the account-key device means losing the ability to
link; account recovery is a later decision.

**History starts at linking.** MLS forward secrecy means a new device cannot
read earlier messages. Transferring history between one's own devices is
a later feature.

**Chats heal themselves.** Rule: every chat holds every registered device of
one's own account.
- The directory's device list is the source of truth:
  `GET /v0/accounts/{username}/devices`.
- Any own device adds a missing sibling to a chat with a key package fetched
  for that one device: `GET /v0/accounts/{username}/devices/{id}/key-package`.
  This is an ordinary sequenced commit, so two devices racing to add the same
  sibling cannot fork the chat; the loser finds nothing left to do.
- A new chat includes the starter's own other devices in its first commit
  (from a lookup of one's own name), so the first message reaches them too.
- A chat where one's own account has spoken counts as introduced on every
  own device; there is no separate "accepted" sync.

## Consequences

- Linking needs both devices online at once, for about a minute.
- A device list in the directory makes the number of devices behind a
  username public. Lookups already revealed it.
- Removing a device (lost phone) needs the same rule in reverse: unregister
  it in the directory, and every own device removes its leaves. That is the
  next slice; the design already points there.
- Reading the same chat on two devices does not yet sync read marks.
