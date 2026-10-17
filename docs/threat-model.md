# Threat model — version 1.0 (closed beta)

**Status:** living document, written for version 1.0. It states what
Zelamar protects, against whom, and where the protection ends, without
rounding up. ADRs give the reasons; `protocol/` gives the exact rules.

## What we protect

| Asset | Where it lives |
|---|---|
| Message content: text, files, voice, reactions, edits, names | Only on the devices in the chat; in transit sealed by MLS (ADR-0003) |
| Who talks to whom | Implied by MLS membership on devices; the server sees only mailbox ids (see below) |
| The account key | Only on the device that created the account (ADR-0009, ADR-0014) |
| History at rest | Each device's store, encrypted under a key sealed by the OS: DPAPI or Android Keystore (ADR-0010, ADR-0013) |

## Adversaries

1. **The service itself**, curious or compromised. It runs our code on
   Cloudflare and sees every request.
2. **The network**: Wi-Fi, ISPs, anyone between a device and Cloudflare.
3. **Another user**: a stranger, a contact, or a group member who turns
   hostile.
4. **Someone holding a device**: lost or stolen, locked or unlocked.
5. **Cloudflare**, as the service's host.

## What each adversary gets

### The service (and Cloudflare)

It **cannot**:
- read any message, file, name of a group, reaction or receipt. Everything
  is MLS ciphertext or a sealed chunk;
- forge a message, add a device to an account, or link a device without
  the code. Certificates are signed by the account key and checked by
  every client (ADR-0009, ADR-0014);
- open a link slot. The key comes from a code it never sees;
- fork a chat on purpose. A sequencer that lies can only stall changes:
  every client verifies commits under MLS.

It **can see**, and this is metadata we accept for v1:
- **Which mailboxes a delivery goes to, and when.** A mailbox belongs to
  one device. Registration ties devices to a username, so the service can
  learn which accounts exchange messages, how often and how much. Hiding
  this needs per-chat mailboxes rotated after joining, plus anonymous
  tokens (ADR-0011, ADR-0012). That is after v1.
- **Sizes:** envelope sizes (padded to 64 bytes inside MLS) and file sizes
  (rounded to 1 MiB chunks, except the last).
- **IP addresses** of every request, including uploads and downloads.
  Rate limits key on them. Our code logs no addresses, but Cloudflare's own
  request logs may record them.
- **How many devices** an account has, and when it links or removes one.
- **Group sizes**, from how many mailboxes a commit or message goes to.

It **can deny service**: drop, delay or refuse anything. Clients retry, and
messages wait in the outbox; nothing is lost on the device.

### The network

It sees TLS to Cloudflare: timing and sizes only. It cannot tell a
message from a file chunk except by size.

### Other users

- **A stranger** who knows a username can start a chat. It arrives as a
  request: until the user accepts, the stranger learns nothing, not even
  that a message arrived (`content.md`). Lookups and claims are
  rate-limited; usernames cost an invite and a proof of work.
- **A contact** sees what is sent in the chat, of course. Their device
  verifies our device certificates, and safety codes let two people check
  that no one sits between them. One limit: a contact's new linked device
  joins a chat silently, as it does in other messengers.
- **A group member** can add people. Nobody can remove others through the
  app, but a modified client could. Everyone would see it and could add
  the person back (ADR-0015).

### Someone holding a device

- **Locked device:** the store is encrypted. Its key is sealed by DPAPI
  (the Windows login) or the Android Keystore (the device's secure
  hardware), so a copied disk image is useless.
- **Unlocked device, or the user's Windows session:** everything on it is
  readable, as in any messenger. A panic PIN and hidden chats come after
  v1.
- **A lost linked device:** the account's first device removes it. It
  then leaves every chat and gets nothing new (ADR-0014). Its local
  history stays on it until it is erased.
- **Losing the device with the account key** means no new devices can be
  linked. Account recovery is a later decision.

## Cryptography in one table

| Purpose | Primitive |
|---|---|
| Chats | MLS (RFC 9420) with ML-KEM-768 + X25519, AES-256-GCM, SHA-384, Ed25519 |
| Identity | Ed25519 account key → device certificates |
| Store at rest | XChaCha20-Poly1305 per row, HKDF-SHA256 from a 32-byte master key |
| Linking | HKDF-SHA256 from a 100-bit code → XChaCha20-Poly1305 |
| Files | ChaCha20-Poly1305 per 1 MiB chunk, fresh key per file |
| Service requests | Ed25519 signatures with domain-separated contexts |

## Out of scope for v1

- Malware on the device, keyloggers, screen capture.
- A global observer correlating timing across the whole internet.
- Hiding which accounts talk to each other from the service (see above).
- Deniability toward the other participants of a chat.
