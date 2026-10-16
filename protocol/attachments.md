# Attachments — version 0

**Status:** draft, normative for protocol version 0. Client: the core's
`attachment` module. Server: `server/src/blob.ts`. Rationale: ADR-0016.
Conventions as in `delivery.md`.

A file travels in two parts:
- its **sealed chunks**, stored by the service as an opaque blob;
- a **`file` message** in the chat, carrying everything needed to fetch
  and open them.

## Sealing

- Every file gets a fresh 32-byte key and a random 16-byte blob id.
- The plaintext is cut into chunks of 1 MiB (1 048 576 bytes); the last one
  may be shorter. An empty file is one empty chunk.
- A file has at most 25 chunks, so at most 25 MiB.
- Chunk `i` of `n` is sealed with ChaCha20-Poly1305 under the file key:

  ```text
  nonce = 0x00 × 8 || u32 i
  aad   = "zelamar/file/v0" || u32 i || u8 (i == n - 1)
  ```

  A sealed chunk is its plaintext plus a 16-byte tag. Swapped, altered or
  missing chunks do not open, and neither does a file cut short: its new
  last chunk was not sealed as last.

## The `file` message

The content kind `file` (`content.md`) has these fields:

| Field | Content |
|---|---|
| `blob` | bytes(16): the blob id |
| `key` | bytes(32): the file key |
| `size` | the plaintext size in bytes, at most 25 MiB |
| `name` | 1–255 bytes of UTF-8, with no `/`, `\` or control characters |
| `mime` | printable ASCII, at most 100 bytes |
| `width`, `height` | optional: image size in pixels, at most 20 000 |
| `thumb` | optional bytes: a JPEG preview of at most 8 KiB |
| `duration_ms` | optional: the length of a voice message or video |

The chunk count follows from `size`. Receivers check every field before
using it. They open the file only after all its chunks have arrived, and
they never let the name choose where the file is saved.

## The blob store

The blob id goes in the path, base64url. The index is the chunk number,
`0` to `24`.

**Uploading:** `PUT /v0/blobs/{blob}/{index}` with:

| Header | Content |
|---|---|
| `Authorization` | `Bearer <the device's mailbox secret>` |
| `Zelamar-Mailbox` | the device's mailbox id (32 bytes, base64url) |
| `Zelamar-Chunks` | the blob's chunk count, 1–25, the same for every chunk |

The body is the sealed chunk: 16 bytes to 1 MiB + 16.

| Answer | When |
|---|---|
| `201` | stored |
| `200` | the same chunk again |
| `409 blob_taken` | other bytes for that chunk, or another chunk count |
| `401 unauthorized` | the mailbox does not exist, or the secret is wrong |
| `429 over_quota` | the mailbox uploaded 512 MiB today (UTC) |
| `429 rate_limited` | too many uploads from this network address |
| `400 malformed` | a bad header or body |
| `413 too_large` | the body is too large |
| `404 unknown_blob` | a bad blob id or index |

**Downloading:** `GET /v0/blobs/{blob}/{index}` returns the sealed chunk as
`application/octet-stream`, or `404 unknown_blob`. It is rate-limited by
network address. It needs no credentials: the id is 128 random bits and
the bytes are sealed.

**Expiry:** a blob is deleted 30 days after its first chunk arrives.
