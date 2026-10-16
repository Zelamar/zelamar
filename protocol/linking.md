# Linking devices — version 0

**Status:** draft, normative for protocol version 0. Client: the core's
`link` module. Server: `server/src/link.ts`. Rationale: ADR-0014.
Conventions as in `delivery.md`.

A device that holds the account key links a new device to the account. The
two devices meet in a short-lived **slot** on the service. Each leaves one
sealed message there. The service cannot open either one.

## The link code

- 20 characters from the lowercase base32 alphabet `a–z2–7`, drawn
  uniformly: 100 random bits.
- Shown as four groups of five: `xxxxx-xxxxx-xxxxx-xxxxx`.
- When reading a typed code, case, dashes and white space are ignored.
- A code is used once. It is never sent anywhere.

From the code (`ikm` = its 20 lowercase ASCII characters):

```text
slot = HKDF-SHA256(salt = "zelamar/link/v0", ikm, info = "slot", 16 bytes)
key  = HKDF-SHA256(salt = "zelamar/link/v0", ikm, info = "key",  32 bytes)
```

## Sealed messages

| Size | Field |
|---|---|
| 2 | magic `ZL` (`0x5A 0x4C`) |
| 1 | version `0` |
| 1 | kind: `1` request, `2` answer |
| 24 | random nonce |
| … | XChaCha20-Poly1305 ciphertext of the CBOR body under `key` |

The associated data is `"zelamar/link/v0" || kind || slot`. A message opens
only as its own kind and only in its own slot.

**Request** (new device → existing device), at most 2 KiB sealed:

| Field | Content |
|---|---|
| `device` | the new device's id (16 bytes) |
| `signature_key` | its Ed25519 MLS signature key (32 bytes) |
| `label` | what it calls itself, at most 40 characters ("Android phone") |

**Answer** (existing device → new device), at most 64 KiB sealed:

| Field | Content |
|---|---|
| `certificate` | the device certificate the account key issued for that device and key (`identity.md`) |
| `username` | the account's username |
| `contacts` | the existing device's contacts: account, username, whether confirmed |
| `introduced` | ids of the chats the account has spoken in |

The new device accepts the answer only if the certificate verifies and
certifies exactly its own device id and key.

**Match code.** Both devices show six digits, `ddd ddd`:

```text
match = u32(SHA-256("zelamar/link/match/v0" || key || request body)[0..4]) mod 1 000 000
```

The person compares them before approving. They differ only if someone else
who saw the code got there first.

## Slots

The slot is `base64url(slot)` in the path. Every route is rate-limited by
network address.

| Request | Answer |
|---|---|
| `PUT /v0/links/{slot}/request`, a sealed request | `201` kept; `200` the same request again; `409 link_taken` another request is there |
| `PUT /v0/links/{slot}/answer`, a sealed answer | `201`, `200` or `409 link_taken` as above; `404 unknown_link` with no request |
| `GET /v0/links/{slot}` | `200 {"request": "…", "answer": "…" or null}`; `404 unknown_link` if empty or expired |
| `DELETE /v0/links/{slot}` | `204`; everything in the slot is gone |

- The service checks only the header and size: `400 malformed`,
  `413 too_large`.
- A slot expires 10 minutes after its request arrives.

## The flow

1. **Existing device:** shows the code and polls `GET` until a request
   appears. It shows the request's label and match code.
2. **New device:** puts its request and polls `GET` until an answer
   appears. It shows the match code meanwhile.
3. **Existing device:** once the person approves, puts the answer.
4. **New device:** creates its store from the answer, registers like any
   device (`directory.md`) and deletes the slot.
5. **Every own device:** brings the new device into its chats (below).

## Chats follow the account

Every chat the account has spoken in holds every registered device of the
account.

- **Listing devices.** `GET /v0/accounts/{username}/devices` returns
  `{"username", "account", "devices": [{"certificate": "…"}]}`. It hands
  out no key package.
- **One device's key package.** `GET /v0/accounts/{username}/devices/{id}`
  returns the lookup shape with that one device. One of its key packages
  is consumed, or the last resort when none is left.
  `404 unknown_device` if there is no such device.
- **Healing.** A device lists its account's devices, then adds the missing
  ones to each chat in one commit per chat. The commit goes through the
  sequencer (`delivery.md`), so two own devices healing at once cannot fork
  a chat.
- **Starting a chat.** A device that starts a chat looks up its own name as
  well. It adds its other devices in the same first commit.
- **Chat requests.** A chat the account has not spoken in yet stays a
  request and is not healed: its other members learn nothing until the
  person accepts.
- **Speaking makes a chat yours.** A message from one's own account makes a
  chat introduced on every own device.
