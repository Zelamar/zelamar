//! Account recovery (ADR-0018, `protocol/recovery.md`).
//!
//! The device holding the account key makes a [`RecoveryKey`] the user
//! writes down, and leaves a kit sealed with it at the service: the account
//! key and what a linked device would learn. A new device that knows the
//! name and the key fetches the kit ([`Restoring`]) and becomes the
//! account's first device again.

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use hkdf::Hkdf;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::random::OpenMlsRand;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::contacts::Contact;
use crate::device::ChatId;
use crate::error::{CoreError, Result};
use crate::identity::{AccountKey, DeviceId};
use crate::link::{cbor, draw_code, from_cbor, read_code, show_code};
use crate::service::{self, Request, normalize_username};

/// Characters in a recovery key, without dashes: 125 random bits.
pub const CODE_LEN: usize = 25;

/// Largest sealed kit the service keeps.
pub const MAX_KIT: usize = 64 * 1024;

const KDF_SALT: &[u8] = b"zelamar/recovery/v0";
const MAGIC: &[u8] = b"ZR";
const VERSION: u8 = 0;
const NONCE_LEN: usize = 24;

/// A recovery key and what follows from it: the key that seals the kit and
/// the token that fetches it.
pub struct RecoveryKey {
    code: Zeroizing<String>,
    key: Zeroizing<[u8; 32]>,
    token: Zeroizing<[u8; 32]>,
}

impl RecoveryKey {
    /// A new recovery key.
    ///
    /// # Errors
    ///
    /// A crypto error.
    pub fn generate() -> Result<Self> {
        Self::from_code(draw_code(CODE_LEN, "drawing a recovery key")?)
    }

    /// Reads a key as people type it: case, dashes and spaces do not matter.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for a mistyped key.
    pub fn parse(text: &str) -> Result<Self> {
        Self::from_code(read_code(text, CODE_LEN, "recovery key")?)
    }

    fn from_code(code: Zeroizing<String>) -> Result<Self> {
        let kdf = Hkdf::<Sha256>::new(Some(KDF_SALT), code.as_bytes());
        let mut key = Zeroizing::new([0u8; 32]);
        let mut token = Zeroizing::new([0u8; 32]);
        kdf.expand(b"key", key.as_mut())
            .and_then(|()| kdf.expand(b"token", token.as_mut()))
            .map_err(|e| CoreError::crypto("deriving recovery keys", format!("{e:?}")))?;
        Ok(Self { code, key, token })
    }

    /// The key as shown: five groups of five.
    #[must_use]
    pub fn display(&self) -> String {
        show_code(&self.code)
    }

    /// The key without dashes, as the store keeps it.
    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    fn token_hash(&self) -> [u8; 32] {
        Sha256::digest(self.token.as_ref()).into()
    }

    fn aad(username: &str) -> Vec<u8> {
        [KDF_SALT, &[0], username.as_bytes()].concat()
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305> {
        XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|e| CoreError::crypto("initializing the recovery cipher", format!("{e:?}")))
    }

    /// `ZR | version | nonce | ciphertext`, bound to `username`.
    fn seal(&self, username: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce: [u8; NONCE_LEN] = RustCrypto::default()
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a nonce", e))?;
        let ciphertext = self
            .cipher()?
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plaintext,
                    aad: &Self::aad(username),
                },
            )
            .map_err(|e| CoreError::crypto("sealing a recovery kit", format!("{e:?}")))?;
        Ok([MAGIC, &[VERSION], &nonce, &ciphertext].concat())
    }

    fn open(&self, username: &str, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        const WHAT: &str = "recovery kit";
        let header = MAGIC.len() + 1;
        if sealed.len() < header + NONCE_LEN || !sealed.starts_with(MAGIC) {
            return Err(CoreError::malformed(WHAT, "not a recovery kit"));
        }
        if sealed[2] != VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: WHAT,
                version: sealed[2].into(),
            });
        }
        let (nonce, ciphertext) = sealed[header..].split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| CoreError::malformed(WHAT, "bad nonce"))?;
        self.cipher()?
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: &Self::aad(username),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| CoreError::Unauthenticated("the recovery kit did not open"))
    }
}

impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryKey(..)")
    }
}

/// What a kit holds: the account key and what a linked device would learn.
#[derive(Serialize, Deserialize)]
pub(crate) struct KitBody {
    #[serde(with = "serde_bytes")]
    pub(crate) account_key: Vec<u8>,
    pub(crate) username: String,
    pub(crate) contacts: Vec<Contact>,
    pub(crate) introduced: Vec<ChatId>,
    #[serde(default)]
    pub(crate) labels: Vec<(DeviceId, String)>,
}

impl Drop for KitBody {
    fn drop(&mut self) {
        self.account_key.zeroize();
    }
}

impl KitBody {
    /// The kit encoded, and a digest that tells whether the service already
    /// holds these very contents.
    pub(crate) fn encode(&self) -> Result<(Zeroizing<Vec<u8>>, [u8; 32])> {
        let plaintext = cbor(self, "recovery kit")?;
        let digest = Sha256::new()
            .chain_update(KDF_SALT)
            .chain_update(&plaintext)
            .finalize()
            .into();
        Ok((plaintext, digest))
    }
}

/// A sealed kit ready to leave at the service, and what it holds.
pub struct KitUpload {
    /// The `PUT`, signed by the account key.
    pub request: Request,
    pub(crate) digest: [u8; 32],
    pub(crate) code: Zeroizing<String>,
}

impl std::fmt::Debug for KitUpload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KitUpload")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

/// Seals `plaintext` (a [`KitBody`]) with `key` for `username`, as a
/// request signed by `account`.
pub(crate) fn upload(
    key: &RecoveryKey,
    account: &AccountKey,
    username: &str,
    (plaintext, digest): (Zeroizing<Vec<u8>>, [u8; 32]),
    now_ms: u64,
) -> Result<KitUpload> {
    let sealed = key.seal(username, &plaintext)?;
    if sealed.len() > MAX_KIT {
        return Err(CoreError::TooLarge {
            what: "recovery kit",
            size: sealed.len(),
            limit: MAX_KIT,
        });
    }
    let request = service::account_signed(
        account,
        "PUT",
        recovery_path(username),
        &json!({
            "token_hash": service::b64(&key.token_hash()),
            "kit": service::b64(&sealed),
        }),
        now_ms,
    )?;
    Ok(KitUpload {
        request,
        digest,
        code: key.code.clone(),
    })
}

fn recovery_path(username: &str) -> String {
    format!("/v0/accounts/{username}/recovery")
}

/// A new device recovering an account: the name and key the user typed.
pub struct Restoring {
    username: String,
    key: RecoveryKey,
}

/// What a recovered device learns from its kit.
pub struct Recovered {
    pub(crate) account: AccountKey,
    pub(crate) username: String,
    pub(crate) contacts: Vec<Contact>,
    pub(crate) introduced: Vec<ChatId>,
    pub(crate) labels: Vec<(DeviceId, String)>,
    pub(crate) key: RecoveryKey,
}

impl Restoring {
    /// Prepares to recover `username` with the key the user typed.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for a bad username or a mistyped key.
    pub fn new(username: &str, code: &str) -> Result<Self> {
        Ok(Self {
            username: normalize_username(username)?,
            key: RecoveryKey::parse(code)?,
        })
    }

    /// Asks the service for the kit.
    ///
    /// # Errors
    ///
    /// An encoding error.
    pub fn fetch(&self) -> Result<Request> {
        Ok(Request {
            method: "POST",
            path: recovery_path(&self.username),
            headers: vec![("content-type", "application/json".to_owned())],
            body: serde_json::to_vec(&json!({ "token": service::b64(self.key.token.as_ref()) }))
                .map_err(|e| CoreError::Encoding {
                    what: "request",
                    source: e.into(),
                })?,
        })
    }

    /// Reads the answer to [`Restoring::fetch`] and opens the kit.
    ///
    /// # Errors
    ///
    /// [`CoreError::Service`] (`wrong_key`) when the name or the key is
    /// wrong, [`CoreError::Unauthenticated`] for a kit this key did not
    /// seal, or [`CoreError::Malformed`].
    pub fn recovered(self, status: u16, body: &[u8]) -> Result<Recovered> {
        #[derive(Deserialize)]
        struct Found {
            kit: String,
        }
        service::expect_success(status, body, &[200])?;
        let found: Found = service::parse(body)?;
        let plaintext = self
            .key
            .open(&self.username, &service::unb64(&found.kit)?)?;
        let kit: KitBody = from_cbor(&plaintext, "recovery kit")?;
        if kit.username != self.username {
            return Err(CoreError::malformed("recovery kit", "for another username"));
        }
        let account = AccountKey::from_secret_bytes(&kit.account_key)?;
        Ok(Recovered {
            account,
            username: self.username,
            contacts: kit.contacts.clone(),
            introduced: kit.introduced.clone(),
            labels: kit.labels.clone(),
            key: self.key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_back_however_it_is_typed() {
        let key = RecoveryKey::generate().expect("key");
        let shown = key.display();
        assert_eq!(shown.len(), CODE_LEN + 4, "five groups of five");
        let typed = shown.to_uppercase().replace('-', " ");
        let again = RecoveryKey::parse(&typed).expect("typed key");
        assert_eq!(again.code(), key.code());
        assert_eq!(*again.key, *key.key);
        assert_eq!(*again.token, *key.token);
        assert_ne!(*key.key, *key.token, "the token never opens the kit");
    }

    #[test]
    fn a_mistyped_key_is_refused() {
        let shown = RecoveryKey::generate().expect("key").display();
        assert!(RecoveryKey::parse(&shown[..shown.len() - 1]).is_err());
        assert!(RecoveryKey::parse(&format!("{shown}a")).is_err());
        assert!(RecoveryKey::parse(&shown.replacen(|c: char| c != '-', "1", 1)).is_err());
    }

    #[test]
    fn a_kit_opens_only_with_its_key_and_name() {
        let key = RecoveryKey::generate().expect("key");
        let sealed = key.seal("alice", b"kit").expect("seal");
        assert_eq!(&*key.open("alice", &sealed).expect("open"), b"kit");
        assert!(key.open("mallory", &sealed).is_err(), "bound to the name");
        let other = RecoveryKey::generate().expect("other key");
        assert!(other.open("alice", &sealed).is_err());
        let mut tampered = sealed;
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(key.open("alice", &tampered).is_err());
    }
}
