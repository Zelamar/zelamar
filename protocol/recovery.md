# Account recovery — version 0

**Status:** draft, normative for protocol version 0. Client: the core's
`recovery` module. Server: `server/src/account.ts`. Rationale: ADR-0018.
Conventions as in `delivery.md`.

The device holding the account key makes a **recovery key** that the person
writes down. It leaves a **kit** at the service, sealed with that key. A new
device that knows the username and the key fetches the kit, opens it, and
becomes the account's first device again. The service cannot open a kit.

## The recovery key

- 25 characters from the lowercase base32 alphabet `a–z2–7`, drawn
  uniformly: 125 random bits.
- Shown as five groups of five: `xxxxx-xxxxx-xxxxx-xxxxx-xxxxx`.
- When reading a typed key, case, dashes and white space are ignored.
- The key never leaves a device. The device that made it keeps it in its
  encrypted store, to show it again.

From the key (`ikm` = its 25 lowercase ASCII characters):

```text
key   = HKDF-SHA256(salt = "zelamar/recovery/v0", ikm, info = "key",   32 bytes)
token = HKDF-SHA256(salt = "zelamar/recovery/v0", ikm, info = "token", 32 bytes)
```

`key` seals the kit; `token` fetches it. The service keeps only
`SHA-256(token)`.

## The kit

| Size | Field |
|---|---|
| 2 | magic `ZR` (`0x5A 0x52`) |
| 1 | version `0` |
| 24 | random nonce |
| … | XChaCha20-Poly1305 ciphertext of the CBOR body under `key` |

The associated data is `"zelamar/recovery/v0" || 0x00 || username`: a kit
opens only for its own name. A sealed kit is at most 64 KiB.

The body is what a linking answer carries (`linking.md`), plus the account
key:

| Field | Content |
|---|---|
| `account_key` | the account's Ed25519 signing key, as the store keeps it |
| `username` | the account's username |
| `contacts` | the account's contacts: account, username, whether confirmed |
| `introduced` | ids of the chats the account has spoken in |
| `labels` | what the account's devices call themselves |

The device holding the account key seals and leaves the kit again whenever
the body changes: a new key, a new contact, a new chat.

## Requests

| Request | Answer |
|---|---|
| `PUT /v0/accounts/{username}/recovery` with `{"token_hash": "<32 bytes>", "kit": "<sealed kit>"}`, signed by the account key | `200 {}`; any earlier kit and token are gone |
| `POST /v0/accounts/{username}/recovery` with `{"token": "<32 bytes>"}` | `200 {"kit": "…"}` when `SHA-256(token)` matches |

- **Leaving a kit** is an account-key request, signed as for removing a
  device (`linking.md`): `Zelamar-Timestamp` and `Zelamar-Account-Signature`,
  no certificate. Errors: `401 unauthorized`, `400 stale_request`,
  `400 malformed`, `413 too_large`, `404 unknown_account`.
- **Fetching a kit.** A wrong token, a name without a kit and a name nobody
  holds all answer `403 wrong_key`, so names cannot be probed for kits.
  `400 malformed` for a token that is not 32 bytes. Fetching is limited to
  10 requests a minute per network address.

## Recovering

1. **The new device** reads the username and the key, fetches the kit and
   opens it. It checks that the kit's username is the one typed.
2. It makes a new device id, MLS signature key and mailbox, and certifies
   them with the recovered account key.
3. It registers like any device (`directory.md`). It now holds the account
   key: it links and removes devices, and keeps the recovery key.
4. **The person removes the lost device** (`linking.md`). Until then the
   lost device holds the account's place in every chat.
5. **The account's chats come back** as their other members heal them
   (below). A chat listed in `introduced` arrives as the account's own, not
   as a request.

Messages from before the recovery stay on the lost device.

## Chats heal across accounts

Every chat an account has spoken in holds exactly the registered devices of
each of its members whose name it knows.

- **Device lists for many names.** `POST /v0/directory/devices` with
  `{"usernames": [...]}`: 1 to 64 normalized names. The answer is
  `{"accounts": [{"username", "account", "devices": [{"certificate": "…"}]}]}`
  for every name someone holds. It hands out no key package. It is
  rate-limited as one lookup. `400 malformed` or `400 invalid_username`
  otherwise.
- **Who heals whom.** An account's own devices heal it while one of its
  devices in the chat is still registered (`linking.md`). When none is, as
  after a recovery, the other members add its registered devices, each from
  `GET /v0/accounts/{username}/devices/{id}`, in one commit through the
  sequencer.
- **Who goes first.** The member whose registered device has the lowest
  leaf in the chat adds them. Any other member does it after the gap has
  lasted an hour. The sequencer settles a race.
- **When.** At most every 5 minutes: on connecting, and while connected.
- **Chat requests** are not healed, as in `linking.md`.
