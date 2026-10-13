//! Encrypted persistence (ADR-0010).
//!
//! One SQLite file per device. Nothing in it is readable without the
//! device's [`MasterKey`]:
//! - every row is addressed by a **slot**: an HMAC of its logical name, so
//!   even chat ids and key ids never appear on disk;
//! - every value is sealed with XChaCha20-Poly1305 under a random nonce,
//!   with the slot as associated data, so rows cannot be swapped or replayed
//!   into other slots;
//! - a sealed canary detects a wrong master key before anything else runs.
//!
//! The store persists two things: the MLS state kept by `OpenMLS` (as a diff,
//! see [`Batch`]) and a handful of named records (device certificate, chat
//! list). All changes of one device operation are applied in one SQLite
//! transaction.

use std::collections::HashMap;
use std::path::Path;

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use openmls_rust_crypto::RustCrypto;
use openmls_traits::random::OpenMlsRand;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{CoreError, Result};

/// Schema version written to `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;
const NONCE_LEN: usize = 24;
const KDF_SALT: &[u8] = b"zelamar/store/v0";
const CANARY: &[u8] = b"zelamar/store-canary/v0";

/// The canary lives in a fixed slot that does not depend on the key: under a
/// wrong key, a keyed slot would simply not be found and a fresh canary would
/// be written instead of reporting the wrong key. (HMAC outputs never
/// realistically collide with all zeros.)
const CANARY_SLOT: [u8; 32] = [0; 32];

/// Namespaces of slots, so equal names in different tables never collide.
const NS_MLS: &[u8] = b"mls";
const NS_RECORD: &[u8] = b"record";

/// The 256-bit key that opens a device's store. The app keeps it in the
/// operating system's secure storage (Android Keystore, Windows DPAPI,
/// iOS Keychain); it never touches the store file.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl MasterKey {
    /// Draws a new random master key.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] if the system random generator fails.
    pub fn generate() -> Result<Self> {
        RustCrypto::default()
            .random_array()
            .map(|bytes| Self(Zeroizing::new(bytes)))
            .map_err(|e| CoreError::crypto("drawing a master key", e))
    }

    /// Restores a master key previously obtained from [`Self::expose`].
    #[must_use]
    pub fn from_bytes(mut bytes: [u8; 32]) -> Self {
        let key = Self(Zeroizing::new(bytes));
        bytes.zeroize();
        key
    }

    /// The raw key, for handing to the OS secure storage. Handle with care.
    #[must_use]
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

/// Keys derived from the master key: one to seal, one to name slots.
struct StoreCipher {
    aead: XChaCha20Poly1305,
    slot_key: Zeroizing<[u8; 32]>,
}

impl StoreCipher {
    fn new(master: &MasterKey) -> Result<Self> {
        let kdf = Hkdf::<Sha256>::new(Some(KDF_SALT), master.expose());
        let mut aead_key = Zeroizing::new([0u8; 32]);
        let mut slot_key = Zeroizing::new([0u8; 32]);
        kdf.expand(b"aead", aead_key.as_mut())
            .and_then(|()| kdf.expand(b"slot", slot_key.as_mut()))
            .map_err(|e| CoreError::crypto("deriving store keys", format!("{e:?}")))?;
        let aead = XChaCha20Poly1305::new_from_slice(aead_key.as_ref())
            .map_err(|e| CoreError::crypto("initializing the store cipher", format!("{e:?}")))?;
        Ok(Self { aead, slot_key })
    }

    /// Deterministic, keyed, non-reversible name of a row.
    fn slot(&self, namespace: &[u8], name: &[u8]) -> Result<[u8; 32]> {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(self.slot_key.as_ref())
            .map_err(|e| CoreError::crypto("initializing the slot MAC", format!("{e:?}")))?;
        mac.update(namespace);
        mac.update(&[0]);
        mac.update(name);
        Ok(mac.finalize().into_bytes().into())
    }

    fn seal(&self, slot: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce: [u8; NONCE_LEN] = RustCrypto::default()
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a nonce", e))?;
        let ciphertext = self
            .aead
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plaintext,
                    aad: slot,
                },
            )
            .map_err(|e| CoreError::crypto("sealing a stored value", format!("{e:?}")))?;
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    fn open(&self, slot: &[u8; 32], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if sealed.len() < NONCE_LEN {
            return Err(CoreError::malformed("stored value", "too short"));
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| CoreError::malformed("stored value", "bad nonce"))?;
        self.aead
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: slot,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| CoreError::Unauthenticated("stored value failed integrity check"))
    }
}

/// Everything one device operation changed, applied atomically.
#[derive(Default)]
pub(crate) struct Batch {
    /// MLS storage entries written or changed: `(key, value)`.
    pub(crate) mls_upserts: Vec<(Vec<u8>, Vec<u8>)>,
    /// MLS storage entries removed.
    pub(crate) mls_deletes: Vec<Vec<u8>>,
    /// Named records written.
    pub(crate) records: Vec<(&'static str, Vec<u8>)>,
}

/// The encrypted SQLite store of one device.
pub(crate) struct Store {
    conn: Connection,
    cipher: StoreCipher,
}

impl Store {
    /// Opens (creating if needed) the store at `path`.
    pub(crate) fn open(path: &Path, master: &MasterKey) -> Result<Self> {
        let conn =
            Connection::open(path).map_err(|e| CoreError::storage("opening the store", e))?;
        // WAL for concurrency-friendly writes; FULL sync so a confirmed
        // MLS state change survives power loss; overwrite deleted data.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA secure_delete = ON;",
        )
        .map_err(|e| CoreError::storage("configuring the store", e))?;
        let store = Self {
            conn,
            cipher: StoreCipher::new(master)?,
        };
        store.migrate()?;
        store.check_canary()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| CoreError::storage("reading the schema version", e))?;
        match version {
            0 => self
                .conn
                .execute_batch(
                    "BEGIN;
                     CREATE TABLE mls_state (slot BLOB PRIMARY KEY NOT NULL, sealed BLOB NOT NULL) WITHOUT ROWID;
                     CREATE TABLE records (slot BLOB PRIMARY KEY NOT NULL, sealed BLOB NOT NULL) WITHOUT ROWID;
                     PRAGMA user_version = 1;
                     COMMIT;",
                )
                .map_err(|e| CoreError::storage("creating the schema", e)),
            SCHEMA_VERSION => Ok(()),
            other => Err(CoreError::UnsupportedVersion {
                what: "store schema",
                version: u16::try_from(other).unwrap_or(u16::MAX),
            }),
        }
    }

    /// Writes the canary on first use; afterwards, proves the master key.
    fn check_canary(&self) -> Result<()> {
        let slot = CANARY_SLOT;
        if let Some(sealed) = self.read_sealed("records", &slot)? {
            return match self.cipher.open(&slot, &sealed) {
                Ok(plaintext) if plaintext.as_slice() == CANARY => Ok(()),
                _ => Err(CoreError::WrongKey),
            };
        }
        let sealed = self.cipher.seal(&slot, CANARY)?;
        self.conn
            .execute(
                "INSERT INTO records (slot, sealed) VALUES (?1, ?2)",
                params![slot.as_slice(), sealed],
            )
            .map(drop)
            .map_err(|e| CoreError::storage("writing the canary", e))
    }

    fn read_sealed(&self, table: &'static str, slot: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        let sql = match table {
            "records" => "SELECT sealed FROM records WHERE slot = ?1",
            _ => "SELECT sealed FROM mls_state WHERE slot = ?1",
        };
        self.conn
            .query_row(sql, params![slot.as_slice()], |row| row.get(0))
            .optional()
            .map_err(|e| CoreError::storage("reading a row", e))
    }

    /// Reads a named record.
    pub(crate) fn record(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let slot = self.cipher.slot(NS_RECORD, name.as_bytes())?;
        self.read_sealed("records", &slot)?
            .map(|sealed| self.cipher.open(&slot, &sealed))
            .transpose()
    }

    /// Loads the complete MLS state.
    pub(crate) fn load_mls_state(&self) -> Result<HashMap<Vec<u8>, Vec<u8>>> {
        let mut statement = self
            .conn
            .prepare("SELECT slot, sealed FROM mls_state")
            .map_err(|e| CoreError::storage("reading the MLS state", e))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|e| CoreError::storage("reading the MLS state", e))?;
        let mut state = HashMap::new();
        for row in rows {
            let (slot, sealed) = row.map_err(|e| CoreError::storage("reading the MLS state", e))?;
            let slot: [u8; 32] = slot
                .try_into()
                .map_err(|_| CoreError::malformed("stored MLS entry", "bad slot"))?;
            let (key, value) = decode_entry(&self.cipher.open(&slot, &sealed)?)?;
            if self.cipher.slot(NS_MLS, &key)? != slot {
                return Err(CoreError::Unauthenticated(
                    "stored MLS entry is in the wrong slot",
                ));
            }
            state.insert(key, value);
        }
        Ok(state)
    }

    /// Applies a batch in a single transaction: all of it or nothing.
    pub(crate) fn apply(&mut self, batch: &Batch) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| CoreError::storage("starting a transaction", e))?;
        for (key, value) in &batch.mls_upserts {
            let slot = self.cipher.slot(NS_MLS, key)?;
            let sealed = self.cipher.seal(&slot, &encode_entry(key, value)?)?;
            tx.execute(
                "INSERT OR REPLACE INTO mls_state (slot, sealed) VALUES (?1, ?2)",
                params![slot.as_slice(), sealed],
            )
            .map_err(|e| CoreError::storage("writing MLS state", e))?;
        }
        for key in &batch.mls_deletes {
            let slot = self.cipher.slot(NS_MLS, key)?;
            tx.execute(
                "DELETE FROM mls_state WHERE slot = ?1",
                params![slot.as_slice()],
            )
            .map_err(|e| CoreError::storage("deleting MLS state", e))?;
        }
        for (name, value) in &batch.records {
            let slot = self.cipher.slot(NS_RECORD, name.as_bytes())?;
            let sealed = self.cipher.seal(&slot, value)?;
            tx.execute(
                "INSERT OR REPLACE INTO records (slot, sealed) VALUES (?1, ?2)",
                params![slot.as_slice(), sealed],
            )
            .map_err(|e| CoreError::storage("writing a record", e))?;
        }
        tx.commit()
            .map_err(|e| CoreError::storage("committing a transaction", e))
    }
}

/// `u32 BE key length || key || value`.
fn encode_entry(key: &[u8], value: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let len = u32::try_from(key.len()).map_err(|_| CoreError::TooLarge {
        what: "MLS storage key",
        size: key.len(),
        limit: u32::MAX as usize,
    })?;
    let mut out = Zeroizing::new(Vec::with_capacity(4 + key.len() + value.len()));
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(value);
    Ok(out)
}

fn decode_entry(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    const WHAT: &str = "stored MLS entry";
    let (len, rest) = bytes
        .split_first_chunk::<4>()
        .ok_or_else(|| CoreError::malformed(WHAT, "too short"))?;
    let len = usize::try_from(u32::from_be_bytes(*len))
        .map_err(|_| CoreError::malformed(WHAT, "key length does not fit"))?;
    if rest.len() < len {
        return Err(CoreError::malformed(WHAT, "key length exceeds entry"));
    }
    let (key, value) = rest.split_at(len);
    Ok((key.to_vec(), value.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(master: &MasterKey) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open(&dir.path().join("device.db"), master).expect("open");
        (dir, store)
    }

    #[test]
    fn records_and_mls_state_round_trip() {
        let master = MasterKey::generate().expect("key");
        let (dir, mut store) = temp_store(&master);
        let batch = Batch {
            mls_upserts: vec![
                (b"k1".to_vec(), b"v1".to_vec()),
                (b"k2".to_vec(), b"v2".to_vec()),
            ],
            mls_deletes: vec![],
            records: vec![("chats", b"abc".to_vec())],
        };
        store.apply(&batch).expect("apply");
        store
            .apply(&Batch {
                mls_deletes: vec![b"k1".to_vec()],
                ..Batch::default()
            })
            .expect("delete");
        drop(store);

        let reopened = Store::open(&dir.path().join("device.db"), &master).expect("reopen");
        let state = reopened.load_mls_state().expect("load");
        assert_eq!(state.len(), 1);
        assert_eq!(state[b"k2".as_slice()], b"v2");
        assert_eq!(
            reopened
                .record("chats")
                .expect("record")
                .as_deref()
                .map(Vec::as_slice),
            Some(b"abc".as_slice())
        );
        assert!(reopened.record("missing").expect("record").is_none());
    }

    #[test]
    fn a_wrong_master_key_is_detected_on_open() {
        let master = MasterKey::generate().expect("key");
        let (dir, store) = temp_store(&master);
        drop(store);
        let other = MasterKey::generate().expect("key");
        assert!(matches!(
            Store::open(&dir.path().join("device.db"), &other),
            Err(CoreError::WrongKey)
        ));
    }

    #[test]
    fn swapped_rows_are_detected() {
        let master = MasterKey::generate().expect("key");
        let (_dir, mut store) = temp_store(&master);
        store
            .apply(&Batch {
                mls_upserts: vec![
                    (b"a".to_vec(), b"1".to_vec()),
                    (b"b".to_vec(), b"2".to_vec()),
                ],
                ..Batch::default()
            })
            .expect("apply");
        let slot_a = store.cipher.slot(NS_MLS, b"a").expect("slot");
        let slot_b = store.cipher.slot(NS_MLS, b"b").expect("slot");
        let sealed_a = store
            .read_sealed("mls_state", &slot_a)
            .expect("read")
            .expect("row");
        store
            .conn
            .execute(
                "UPDATE mls_state SET sealed = ?1 WHERE slot = ?2",
                params![sealed_a, slot_b.as_slice()],
            )
            .expect("tamper");
        assert!(matches!(
            store.load_mls_state(),
            Err(CoreError::Unauthenticated(_))
        ));
    }

    #[test]
    fn master_key_debug_hides_the_key() {
        let master = MasterKey::from_bytes([42; 32]);
        assert_eq!(format!("{master:?}"), "MasterKey(..)");
        assert_eq!(master.expose(), &[42; 32]);
    }
}
