# 0010. Encrypted device store: SQLite + per-row AEAD over MLS state diffs

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

A device must survive restarts: its keys, its MLS state and its chat list
must be persisted. The roadmap promises that nothing is readable on disk
without the user's key. The prototype stored private keys and message
previews in plaintext.

Options considered:

1. **SQLCipher** (whole-file encryption, used by Signal Desktop). Its only
   crypto backend supported by `libsqlite3-sys` is OpenSSL. A probe build on
   the CEO's Windows PC failed: the vendored OpenSSL needs a native Perl, and
   Android cross-builds add NDK friction. Rejected for developer and CI cost.
2. **`openmls_sqlite_storage`**: writes MLS keys and values in plaintext
   through a static, instance-less codec, so encryption cannot be added
   cleanly. Rejected.
3. **Forking `openmls_memory_storage` onto an encrypted backend**: about 80
   methods to maintain across `OpenMLS` upgrades. Rejected in favour of 4.
4. **Official `MemoryStorage` + encrypted diff persistence.** `OpenMLS` keeps
   using its own in-memory storage unchanged. After each device operation
   the core diffs the storage map against what is on disk and writes only
   the changes, encrypted, in one SQLite transaction.

## Decision

Option 4, in `core/zelamar-core/src/store.rs` and `device.rs`:

- **Keys.** A 256-bit `MasterKey` is held by the app in OS secure storage.
  HKDF-SHA256 derives an AEAD key and a slot key.
- **Rows.** Every row is addressed by a slot: HMAC-SHA256 of
  `namespace || 0 || logical name`, so chat ids, key ids and record names
  never reach the disk. Values are sealed with XChaCha20-Poly1305 under a
  random nonce, with the slot as associated data, so a row moved to another
  slot fails authentication. MLS rows also carry their original key inside
  the ciphertext and are re-checked against the slot on load.
- **Wrong key.** A canary in a fixed, key-independent slot reports
  `WrongKey` before anything else runs. A keyed slot would silently miss; a
  test caught exactly that bug.
- **Atomic units of work.** Every device operation runs in `transact`:
  - on success, the diff and changed records commit in one transaction;
  - on any failure, the in-memory MLS state and the loaded groups are
    restored from what is on disk.
  
  A rejected message therefore changes nothing, even across a restart.
- **Durability.** WAL mode, `synchronous = FULL` (a confirmed MLS state
  change survives power loss), `secure_delete = ON`. The schema is versioned
  through `PRAGMA user_version`.
- `rusqlite` is pinned to 0.37 because `openmls` declares an optional
  `openmls_sqlite_storage` (rusqlite 0.37), and Cargo allows only one
  `sqlite3`-linking crate in the graph.

## Consequences

- No C crypto dependency; builds identically on Windows, Android and iOS.
- The whole MLS state lives in memory while the app runs. That is fine for a
  personal messenger (kilobytes to low megabytes). Each operation's diff
  costs O(entries) comparisons.
- Visible on disk: the number and sizes of rows, and the time of writes.
  Not visible: any name, id, key or content.
- The device that creates the account also stores the account root key
  (sealed). Device linking (Phase 4) builds on it.
- Tests: wrong key, swapped rows, restart mid-conversation, a key package
  surviving its owner's restart, rollback after a rejected message, and a
  scan of every file on disk for plaintext, ids and record names.
