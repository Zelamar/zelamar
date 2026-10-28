# 0020. Drift D1: messages over the local network when the internet is gone

- **Status:** Accepted
- **Date:** 2026-10-28

## Context

Messages without internet are the third item of version 2 (CEO decision,
2026-10-26). The first step of Drift (ROADMAP 6.2, D1) is the local
network: a Wi-Fi router whose internet is down, an office, a campus, a
ship. Every device there can still reach the others.

Envelopes were made for this from the first day (ADR-0004):
- they are self-contained and encrypted;
- they are addressed to a mailbox, not to a server;
- duplicates are harmless.

Constraints:
- **No new cryptography.** The content stays MLS end to end. A neighbour on
  the network must learn as little as possible.
- **One way to do things.** No setting to turn on. When the internet is
  gone, nearby delivery simply works.
- **Windows and Android first,** with the same code on iPhone later.

## Decision

**Only while the delivery service is out of reach.**
- After 10 seconds without a connection to the service, a device starts
  looking for neighbours. It stops when the connection is back.
- With the internet there, nothing about the device shows on the local
  network.

**Finding neighbours: a UDP beacon, no identity.**
- Every 3 seconds the device sends a small multicast datagram to
  `239.77.65.82:47321`: magic, version, a random session id, and the TCP
  port it listens on.
- A new session id is drawn each time nearby delivery starts. The beacon
  says only "a Zelamar is here".
- On Android, a multicast lock lets the Wi-Fi chip pass the beacons in.

**Handing over envelopes: a short TCP exchange.**
1. The device that has something to send connects to a neighbour's port.
2. The neighbour sends a random 16-byte nonce.
3. The sender asks about the mailboxes it holds envelopes for, as tags:
   `HKDF-SHA256(salt = nonce, ikm = mailbox id, info = "zelamar/lan/v0")`,
   16 bytes each.
4. The neighbour answers with the tags of its own mailboxes. A tag tells
   nothing about any other mailbox, and changes with every nonce.
5. The sender sends only the envelopes for those mailboxes.
6. The neighbour opens each one like any envelope from the service and
   answers with its id.

**What travels.**
- Ordinary messages travel this way, and so do their receipts, edits,
  reactions, calls' end lines and names.
- Changes to a chat's membership (commits, welcomes) wait for the service,
  which orders them (ADR-0012).
- Files wait too: their chunks live at the service.

**The outbox does not change.**
- A message handed over nearby stays in the outbox. It goes through the
  service too once the internet is back, so the other devices of that
  person get it, and so does anyone not nearby.
- The copy that arrives second is refused by MLS as already read: it
  shows nothing twice.

## Consequences

- Two people in a room without internet can write to each other, with the
  same encryption as always, as long as they already share a chat.
- Starting a chat, adding people and sending files still need the
  internet once.
- On a network without internet, other people can see that a Zelamar
  device is there, though not whose it is. Someone who knows one of its
  mailbox ids can ask about it and recognize it. Its chats know the id,
  and so does anyone who looks its username up in the directory: the id
  is in its key packages. Rotating mailboxes per chat (ADR-0011) would
  close this; until then the threat model says so.
- Next steps of Drift build on the same exchange:
  - D2: phone to phone over Bluetooth or Wi-Fi Direct;
  - D3: envelopes carried by other devices.
