# 0018. Account recovery: a key only the user holds, and chats that heal across accounts

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

The account is its key, and the key lives on the device that created the
account (ADR-0009, ADR-0014). Losing that device loses the account: the
name, the contacts, every chat. For the closed beta most people have one
phone, so a lost or broken phone is the likeliest way to lose Zelamar.

Constraints:
- **No phone number or email** to fall back on, and no one at Zelamar who
  can vouch for anybody.
- **The service must learn nothing it could use** to take an account, even
  if it is malicious.
- **No passkeys yet.** They need a domain Zelamar owns, and the PRF
  extension that would turn one into a key is not everywhere yet.
- **Forward secrecy.** MLS cannot give a new device earlier messages, and a
  copy of an old device's chat state must never be reused.

## Decision

**A recovery key the user writes down.**
- *You → Recovery key* on the device holding the account key creates it:
  25 letters and digits in five groups of five, like link codes
  (`xxxxx-xxxxx-xxxxx-xxxxx-xxxxx`). It is 125 random bits.
- That device keeps it in its encrypted store, so the user can see it again.
- *Replace key* draws a new one; the old one stops working at once.
- Until a key exists, the chat list shows a quiet reminder.

**The service keeps a sealed recovery kit and a lock for it.**
- Both come from the key with HKDF-SHA256 (salt `zelamar/recovery/v0`):
  - a 32-byte sealing key;
  - a 32-byte fetch token.
- The kit is XChaCha20-Poly1305 over the account key, the username, the
  contacts, the chats the account has spoken in and the device labels: what
  a linking answer carries, plus the account key. Its associated data binds
  it to the username.
- `PUT /v0/accounts/{name}/recovery`, signed by the account key, stores
  `{token_hash, kit}`. The service keeps only SHA-256 of the token. Kits are
  at most 64 KiB.
- `POST /v0/accounts/{name}/recovery` with `{token}` returns the kit when the
  token's hash matches. It is rate-limited per address.
- The device holding the account key seals and uploads the kit again
  whenever its contents change.

**Recovering is a new device that holds the account key.**
- *I already use Zelamar → Use a recovery key*: the user types their name and
  the key.
- The new device fetches and opens the kit, then certifies itself:
  - a new device id, MLS signature key and mailbox;
  - registered under the name like any other device.
- From then on it is the account's first device: it links and removes
  devices.
- The app opens *Devices* next, asking the user to remove the devices they
  no longer have.

**Chats heal across accounts.** ADR-0014 heals one's own devices. The rule
now covers every member whose name is known: every chat holds exactly the
registered devices of each of its accounts.
- **One's own devices** heal their account, as before.
- **The other members** heal an account only when none of its devices left
  in the chat is still registered, as after a recovery. In that case the
  member with the lowest registered leaf:
  - removes the account's unregistered devices;
  - then adds its registered ones.
- **If that member stays away**, any member that has seen the same gap for an
  hour acts instead. The sequencer settles any race (ADR-0012).
- **One request** fetches the device lists of up to 64 names:
  `POST /v0/directory/devices`. It is rate-limited as a single lookup, and
  an app sends it at most every 5 minutes.
- **A recovered device accepts these invitations quietly** for the chats its
  kit lists. Everything else arrives as a chat request, as usual.

**History before the recovery stays on the lost device.** MLS forward
secrecy means chats cannot hand it over. An encrypted history backup is a
later decision.

## Consequences

- **The recovery key is the account.** Whoever holds it can take the name,
  link devices and be added to every chat. The app says so plainly: keep it
  like a password, offline.
- **The service never sees the key.** A malicious service holding a kit
  would need to guess 125 bits. Fetching one needs the token, so kits cannot
  be collected by name.
- **Contacts get no warning** when an account they talk to gains a device.
  A "new device" notice in the chat is a later feature.
- **Without a key, losing the account-key device still loses the account.**
  That is why the reminder stays until a key exists.
- **The lost phone keeps receiving until it is removed.** Until the user
  removes it, it holds the account's only registered leaf in each chat, so
  contacts do not heal the account.
- **An account at its five-device limit** must remove a device before it can
  recover.
- **Healing costs one more request** per connection, at most every 5 minutes.
  More than 64 names take more requests.
