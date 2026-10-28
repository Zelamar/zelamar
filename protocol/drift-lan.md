# Drift D1: the local network — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/lan.rs` (what travels, tags) and
`app/lib/src/drift/lan.dart` (beacons, exchanges). Decision: ADR-0020.

While a device cannot reach the delivery service, it hands envelopes
(`envelope.md`) to devices on the same local network that host their
mailboxes. Nothing else changes: envelopes stay in the outbox until the
service has them too, and a copy that arrives twice is refused by MLS.

## When

- Nearby delivery starts after 10 seconds without a connection to the
  service.
- It stops as soon as the connection is back.
- Each start draws a new 16-byte session id at random.

## Beacons

Every 3 seconds, a UDP datagram to the multicast group `239.77.65.82`,
port `47321`:

| Size | Field |
|---|---|
| 2 | magic `ZL` (`0x5A 0x4C`) |
| 1 | version `0` |
| 16 | session id |
| 2 | TCP port the device listens on, big-endian |

A beacon names no one. A device ignores its own beacons, and anything
that is not exactly 21 bytes with the right magic and version.

## Exchanges

A device with envelopes waiting connects to a neighbour's TCP port. It
does so at most every 10 seconds per neighbour, and at once after it
queues something new.

Frames are kind (1 byte), length (4 bytes, big-endian), bytes. The
largest frame is 55 + 1 048 576 bytes.

| Kind | Name | Bytes | From |
|---|---|---|---|
| 1 | hello | a 16-byte random nonce | listener |
| 2 | ask | 1 to 255 tags of 16 bytes | sender |
| 3 | have | the tags among those it hosts, possibly none | listener |
| 4 | envelope | one envelope | sender |
| 5 | taken | the envelope's 16-byte id | listener |
| 6 | done | nothing | sender |

The sender's steps:
1. Read `hello`.
2. Send `ask` with the tags of the mailboxes it holds envelopes for, or
   `done` if it has none.
3. Read `have`.
4. Send at most 256 `envelope` frames, for the mailboxes in `have` only,
   then `done`.
5. Read one `taken` per envelope.

The listener answers each `envelope` with `taken` once it has opened it,
whether or not MLS accepted it.

A mailbox's tag under a nonce:

    tag = HKDF-SHA256(salt = nonce, ikm = mailbox id, info = "zelamar/lan/v0")[0..16]

## What travels

- MLS application messages only: texts, replies, edits, deletions,
  reactions, receipts, names, calls' offers and ends.
- Commits and welcomes wait for the service, which orders them. File
  chunks wait too: they live at the service.
- A sender does not offer a neighbour an envelope that neighbour already
  took in this session.

## Rules for a listener

1. Any frame of an unexpected kind, size or order ends the exchange.
2. A step that takes longer than 10 seconds ends the exchange.
3. An envelope is opened exactly like one from the service. One addressed
   to another mailbox is refused.
