# 0016. Attachments: encrypted chunks in Durable Objects, keys in the message

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

Photos, files and voice messages must work in version 1.0. They are too
big for an MLS message: content is capped at 64 KiB (`protocol/content.md`).

Constraints:
- **Budget.** $0 on the Workers Free plan (ADR-0012): 5 GB of Durable
  Object storage, 100k rows written per day, and SQLite values of at most
  2 MB. R2 would mean enabling a product and putting a card on file.
- **Privacy.** The server must not learn content, names, types or who
  sends a file to whom.
- **Abuse.** Free storage must not be open to anyone on the internet.

## Decision

**Encrypt on the device; the server stores opaque chunks.**
- Each file gets a fresh 32-byte key and a random 16-byte blob id.
- The plaintext is cut into 1 MiB chunks. Each chunk is sealed with
  ChaCha20-Poly1305 under the file key:
  - nonce = the chunk index as 12 bytes;
  - associated data = `"zelamar/file/v0" || u32 index || u8 last`.

  So chunks cannot be reordered, and the file cannot be cut short.
- A file is at most 25 MiB, that is, 25 chunks.

**The message carries the capability.** A new content kind, `file`, holds:
- the blob id, key, size and chunk count;
- the file name and media type;
- for images, width, height and a small JPEG thumbnail (at most 8 KiB)
  shown at once;
- for voice messages, the duration.

Everything sensitive is inside the MLS message. A device keeps it in its
encrypted history and can fetch the file again until it expires.

**A `Blob` Durable Object per blob id.**
- `PUT /v0/blobs/{id}/{index}` stores one chunk. The client declares the
  chunk count with the first chunk. Each index is written once; repeating
  the same bytes is harmless.
- `GET /v0/blobs/{id}/{index}` returns a chunk to anyone who knows the id:
  128 random bits, and useless without the key from the message.
- A blob is deleted 30 days after its first chunk, like undelivered mail.

**Only registered devices upload.** An upload names a mailbox and proves
its secret, as reading a mailbox does. Uploads are also rate-limited per
mailbox and per network address. The server learns that a mailbox uploaded
some bytes, never what they are, nor who downloads them.

**The app handles the files.**
- It encrypts and uploads before it sends the message; the outbox holds
  only the message.
- On the receiving side, images and voice messages download on arrival.
  Other files download on tap.
- Decrypted files live in the app's own data directory, never in shared
  storage, until the user saves one.

**Voice messages are files.**
- They are recorded as AAC-LC in an MP4 container (`audio/mp4`): mono,
  22.05 kHz, 32 kbit/s, about 240 KB a minute, at most 5 minutes.
- `duration_ms` tells a voice message from any other audio file.
- On playback, the decrypted audio lives in a temporary file only while its
  bubble is on screen, because the players read files.
- Recording asks for the microphone the first time, and only then.

## Consequences

- No new product and no cost. The 5 GB cap, divided by 30 days of
  retention, is ample for a closed beta.
- A file arrives only while its blob lives: after 30 days, recipients who
  never fetched it see it as expired.
- Thumbnails make images appear instantly, at up to 8 KiB per message.
- Upload quotas per account are a later step, alongside anonymous tokens
  (ADR-0012).
