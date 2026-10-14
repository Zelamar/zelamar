# Directory — version 0

**Status:** draft, normative for protocol version 0. Server:
`server/src/`. Rationale: ADR-0012. Conventions as in `delivery.md`.

The directory maps an `@username` to an account key and its devices, and
hands out one-time key packages so anyone can start a chat with a device
that is offline.

## Usernames

- 3–32 characters from `a–z`, `0–9` and `_`.
- The first character is a letter. No `__`, and no `_` at the end.
- Input is lowercased before any check; only the lowercased form is stored.
- Reserved names (`zelamar`, `admin`, `support`, `security`, …) cannot be
  claimed. The list lives in `server/src/username.ts`.
- There is exact lookup only. Nothing lists, searches or suggests names.

## Claiming a username

`POST /v0/accounts` with JSON:

| Field | Content |
|---|---|
| `username` | the name |
| `account` | the account public key (32 bytes) |
| `invite` | a closed-beta invite code |
| `work` | 8 bytes: the proof of work |
| `timestamp` | UTC milliseconds |
| `signature` | Ed25519 by the account key over the claim payload |

```text
claim payload = "zelamar/claim/v0" || u8 len || username || u64 timestamp || work
proof of work: SHA-256("zelamar/work/v0" || account || u8 len || username || work)
               starts with at least 21 zero bits
```

The difficulty is the service's `WORK_BITS`: 21 in production, about a second
or two on a phone. Test deployments may set it lower.

Rules:
- The timestamp is within 5 minutes of the server clock.
- Invite codes are single-use.
- Claiming the same name again with the same account key is a harmless
  retry: `200`.

Responses:
- `201` success;
- `400 invalid_username`, `400 insufficient_work`, `400 stale_request`;
- `401 bad_signature`, `403 invalid_invite`, `403 reserved`;
- `409 username_taken`.

## Signed device requests

Requests on behalf of a device carry three headers:

| Header | Content |
|---|---|
| `Zelamar-Certificate` | the device certificate (`identity.md`) |
| `Zelamar-Timestamp` | UTC milliseconds, within 5 minutes of the server clock |
| `Zelamar-Signature` | Ed25519 by the certified device key |

```text
signed = "zelamar/request/v0" || method || 0x00 || path || 0x00
         || u64 timestamp || SHA-256(body)
```

The server accepts the request only if all of the following hold:
1. the certificate verifies under the account key registered for the
   username in the path;
2. it certifies the device id in the path;
3. the signature verifies under the certified device key.

Replays inside the window are harmless: every signed request is
idempotent.

Errors common to signed requests:
- `401 unauthorized`: the certificate, device id or signature does not check out;
- `400 stale_request`;
- `404 unknown_account`;
- `413 too_large`: the body is over 3 MiB.

## Registering a device

`PUT /v0/accounts/{username}/devices/{device}`, signed, with JSON:

| Field | Content |
|---|---|
| `mailbox` | the device's mailbox id (32 bytes) |
| `mailbox_secret_hash` | `SHA-256(secret)` of its 32-byte mailbox secret |
| `key_packages` | one-time key packages (TLS-encoded), 0–100 |
| `last_resort` | a last-resort key package, reused once one-time ones run out |

Rules:
- At most 5 devices per account.
- A mailbox already claimed with a different secret hash fails with
  `409 mailbox_taken`.
- Success: `200 {"key_packages": <stored count>}`.
- Errors: `400 malformed`, `409 mailbox_taken`, `409 too_many_devices`.

## Publishing key packages

`POST /v0/accounts/{username}/devices/{device}/key-packages`, signed, with
JSON `{"key_packages": [...], "last_resort": ...?}`.

- Duplicates are ignored. Each key package is at most 16 KiB, and at most
  100 are stored per device.
- Responds `200 {"key_packages": <stored count>}`, or `404 unknown_device`
  if the device is not registered.
- A device replenishes when the count runs low.

## Looking up an account

`GET /v0/accounts/{username}` responds with:

```json
{"username": "alice", "account": "<32 bytes>", "devices": [{"certificate": "…", "key_package": "…"}]}
```

- Each device's oldest one-time key package is handed out and deleted.
  When none is left, its last-resort key package is returned instead.
- A device with no key package at all is omitted.
- The client verifies everything it receives, as for any member
  (`addressing.md`). It never trusts the directory for identity.
- `404 unknown_account` if there is no such account.
- Lookups will spend an anonymous token (ADR-0012). Until then they are
  rate-limited by network address.
- The client checks that `username` is the name it asked for.

## Looking up only the account key

`GET /v0/accounts/{username}/key` responds with
`{"username": "alice", "account": "<32 bytes>"}` and hands out no key
package. Clients use it to confirm a username someone claims in a chat
(`content.md`, `profile`). It is rate-limited like a lookup.
