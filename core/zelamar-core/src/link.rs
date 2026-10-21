//! Linking a new device to an account (ADR-0014, `protocol/linking.md`).
//!
//! The device holding the account key shows a [`Offer`]'s code; the new
//! device types it into a [`Joining`]. Both derive the same rendezvous slot
//! and key from the code, so the two sealed messages they leave in the slot
//! mean nothing to the service.

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use hkdf::Hkdf;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::random::OpenMlsRand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::contacts::Contact;
use crate::device::{CIPHERSUITE, ChatId};
use crate::error::{CoreError, Result};
use crate::identity::{DeviceCertificate, DeviceId};
use crate::service::{self, Request};

/// Characters in a link code, without dashes: 100 random bits.
pub const CODE_LEN: usize = 20;

/// Largest sealed request the service keeps.
pub const MAX_REQUEST: usize = 2 * 1024;

/// Largest sealed answer the service keeps.
pub const MAX_ANSWER: usize = 64 * 1024;

/// Longest device label, in characters.
pub const MAX_LABEL: usize = 40;

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const KDF_SALT: &[u8] = b"zelamar/link/v0";
const MATCH_CONTEXT: &[u8] = b"zelamar/link/match/v0";
const MAGIC: &[u8] = b"ZL";
const VERSION: u8 = 0;
const NONCE_LEN: usize = 24;

/// Which of the two messages a sealed blob is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Kind {
    Request = 1,
    Answer = 2,
}

/// What the code gives both devices: the slot to meet in and the key.
struct LinkSecret {
    code: Zeroizing<String>,
    slot: [u8; 16],
    key: Zeroizing<[u8; 32]>,
}

/// `len` (at most 32) random characters of the code alphabet: link codes
/// and recovery keys.
pub(crate) fn draw_code(len: usize, what: &'static str) -> Result<Zeroizing<String>> {
    let random: Zeroizing<[u8; 32]> = Zeroizing::new(
        RustCrypto::default()
            .random_array()
            .map_err(|e| CoreError::crypto(what, e))?,
    );
    // 32 divides 256, so every character is uniform.
    Ok(Zeroizing::new(
        random
            .iter()
            .take(len)
            .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
            .collect(),
    ))
}

/// Reads a code of `len` characters as people type it: case, dashes and
/// spaces do not matter.
pub(crate) fn read_code(text: &str, len: usize, what: &'static str) -> Result<Zeroizing<String>> {
    let code: Zeroizing<String> = Zeroizing::new(
        text.chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .map(|c| c.to_ascii_lowercase())
            .collect(),
    );
    if code.len() != len || !code.bytes().all(|b| ALPHABET.contains(&b)) {
        return Err(CoreError::malformed(
            what,
            "not the right number of letters and digits",
        ));
    }
    Ok(code)
}

/// A code as shown: groups of five joined by dashes.
pub(crate) fn show_code(code: &str) -> String {
    code.as_bytes()
        .chunks(5)
        .map(|group| String::from_utf8_lossy(group).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

impl LinkSecret {
    fn generate() -> Result<Self> {
        Self::from_code(draw_code(CODE_LEN, "drawing a link code")?)
    }

    fn parse(text: &str) -> Result<Self> {
        Self::from_code(read_code(text, CODE_LEN, "link code")?)
    }

    fn from_code(code: Zeroizing<String>) -> Result<Self> {
        let kdf = Hkdf::<Sha256>::new(Some(KDF_SALT), code.as_bytes());
        let mut slot = [0u8; 16];
        let mut key = Zeroizing::new([0u8; 32]);
        kdf.expand(b"slot", &mut slot)
            .and_then(|()| kdf.expand(b"key", key.as_mut()))
            .map_err(|e| CoreError::crypto("deriving link keys", format!("{e:?}")))?;
        Ok(Self { code, slot, key })
    }

    /// The code as shown: four groups of five.
    fn display(&self) -> String {
        show_code(&self.code)
    }

    fn path(&self) -> String {
        format!("/v0/links/{}", service::b64(&self.slot))
    }

    fn aad(&self, kind: Kind) -> Vec<u8> {
        [KDF_SALT, &[kind as u8], &self.slot].concat()
    }

    fn cipher(&self) -> Result<XChaCha20Poly1305> {
        XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|e| CoreError::crypto("initializing the link cipher", format!("{e:?}")))
    }

    /// `ZL | version | kind | nonce | ciphertext`.
    fn seal(&self, kind: Kind, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce: [u8; NONCE_LEN] = RustCrypto::default()
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a nonce", e))?;
        let ciphertext = self
            .cipher()?
            .encrypt(
                &nonce.into(),
                Payload {
                    msg: plaintext,
                    aad: &self.aad(kind),
                },
            )
            .map_err(|e| CoreError::crypto("sealing a link message", format!("{e:?}")))?;
        Ok([MAGIC, &[VERSION, kind as u8], &nonce, &ciphertext].concat())
    }

    fn open(&self, kind: Kind, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        const WHAT: &str = "link message";
        let header = MAGIC.len() + 2;
        if sealed.len() < header + NONCE_LEN || !sealed.starts_with(MAGIC) {
            return Err(CoreError::malformed(WHAT, "not a link message"));
        }
        if sealed[2] != VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: WHAT,
                version: sealed[2].into(),
            });
        }
        if sealed[3] != kind as u8 {
            return Err(CoreError::malformed(WHAT, "the wrong kind"));
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
                    aad: &self.aad(kind),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| CoreError::Unauthenticated("a link message did not open"))
    }

    /// Six digits both screens show for the same request.
    fn match_code(&self, request: &[u8]) -> String {
        let digest = Sha256::new()
            .chain_update(MATCH_CONTEXT)
            .chain_update(self.key.as_ref())
            .chain_update(request)
            .finalize();
        let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
        format!("{:03} {:03}", number / 1000, number % 1000)
    }

    fn put(&self, part: &str, sealed: Vec<u8>) -> Request {
        Request {
            method: "PUT",
            path: format!("{}/{part}", self.path()),
            headers: vec![("content-type", "application/octet-stream".to_owned())],
            body: sealed,
        }
    }

    fn get(&self) -> Request {
        Request {
            method: "GET",
            path: self.path(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Reads the slot: what is there, still sealed.
    fn slot(status: u16, body: &[u8]) -> Result<Option<Stored>> {
        if status == 404 {
            return Ok(None);
        }
        service::expect_success(status, body, &[200])?;
        let found: Found = service::parse(body)?;
        Ok(Some(Stored {
            request: service::unb64(&found.request)?,
            answer: found.answer.as_deref().map(service::unb64).transpose()?,
        }))
    }
}

#[derive(Deserialize)]
struct Found {
    request: String,
    answer: Option<String>,
}

struct Stored {
    request: Vec<u8>,
    answer: Option<Vec<u8>>,
}

/// What the new device tells the existing one.
#[derive(Serialize, Deserialize)]
struct RequestBody {
    device: DeviceId,
    #[serde(with = "serde_bytes")]
    signature_key: Vec<u8>,
    label: String,
}

/// What the existing device hands the new one.
#[derive(Serialize, Deserialize)]
pub(crate) struct AnswerBody {
    #[serde(with = "serde_bytes")]
    pub(crate) certificate: Vec<u8>,
    pub(crate) username: String,
    pub(crate) contacts: Vec<Contact>,
    pub(crate) introduced: Vec<ChatId>,
    /// What the account's devices call themselves.
    #[serde(default)]
    pub(crate) labels: Vec<(DeviceId, String)>,
}

pub(crate) fn cbor<T: Serialize>(value: &T, what: &'static str) -> Result<Zeroizing<Vec<u8>>> {
    let mut out = Zeroizing::new(Vec::new());
    ciborium::into_writer(value, &mut *out).map_err(|e| CoreError::Encoding {
        what,
        source: e.into(),
    })?;
    Ok(out)
}

pub(crate) fn from_cbor<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    what: &'static str,
) -> Result<T> {
    ciborium::from_reader(bytes).map_err(|_| CoreError::malformed(what, "invalid encoding"))
}

/// The existing device's side of a link: a fresh code, and the new device
/// that answers to it.
pub struct Offer {
    secret: LinkSecret,
}

/// A device asking to be linked, as the existing device sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applicant {
    /// The device's id.
    pub device: DeviceId,
    /// What the device calls itself ("Android phone").
    pub label: String,
    /// The six digits the new device shows too.
    pub match_code: String,
    pub(crate) signature_key: Vec<u8>,
}

impl Offer {
    /// A new offer with a fresh code.
    ///
    /// # Errors
    ///
    /// A crypto error.
    pub fn new() -> Result<Self> {
        Ok(Self {
            secret: LinkSecret::generate()?,
        })
    }

    /// The code to show, as `xxxxx-xxxxx-xxxxx-xxxxx`.
    #[must_use]
    pub fn code(&self) -> String {
        self.secret.display()
    }

    /// Asks the service what is in the slot.
    #[must_use]
    pub fn check(&self) -> Request {
        self.secret.get()
    }

    /// Reads the answer to [`Offer::check`]: the new device, once it asked.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a request not sealed with this
    /// code, [`CoreError::Malformed`] or [`CoreError::Service`].
    pub fn applicant(&self, status: u16, body: &[u8]) -> Result<Option<Applicant>> {
        let Some(stored) = LinkSecret::slot(status, body)? else {
            return Ok(None);
        };
        let plaintext = self.secret.open(Kind::Request, &stored.request)?;
        let request: RequestBody = from_cbor(&plaintext, "link request")?;
        if request.signature_key.len() != 32 || request.label.chars().count() > MAX_LABEL {
            return Err(CoreError::malformed("link request", "bad key or label"));
        }
        Ok(Some(Applicant {
            device: request.device,
            label: request.label,
            match_code: self.secret.match_code(&plaintext),
            signature_key: request.signature_key,
        }))
    }

    /// Seals `answer` into the slot.
    pub(crate) fn answer(&self, answer: &AnswerBody) -> Result<Request> {
        let sealed = self
            .secret
            .seal(Kind::Answer, &cbor(answer, "link answer")?)?;
        if sealed.len() > MAX_ANSWER {
            return Err(CoreError::TooLarge {
                what: "link answer",
                size: sealed.len(),
                limit: MAX_ANSWER,
            });
        }
        Ok(self.secret.put("answer", sealed))
    }
}

/// The new device's side of a link: its keys, made before it belongs to
/// any account, and the request carrying them.
pub struct Joining {
    secret: LinkSecret,
    pub(crate) signer: SignatureKeyPair,
    pub(crate) device: DeviceId,
    request: Zeroizing<Vec<u8>>,
}

/// What a linked device learns from the existing one.
pub struct Linked {
    pub(crate) certificate: DeviceCertificate,
    pub(crate) username: String,
    pub(crate) contacts: Vec<Contact>,
    pub(crate) introduced: Vec<ChatId>,
    pub(crate) labels: Vec<(DeviceId, String)>,
}

impl Joining {
    /// Prepares to join the account behind `code`, as a device called
    /// `label`.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for a mistyped code or an overlong label,
    /// or a crypto error.
    pub fn new(code: &str, label: &str) -> Result<Self> {
        let secret = LinkSecret::parse(code)?;
        if label.chars().count() > MAX_LABEL {
            return Err(CoreError::malformed("device label", "too long"));
        }
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|e| CoreError::crypto("generating the device signature key", e))?;
        let device = DeviceId::random()?;
        let request = cbor(
            &RequestBody {
                device,
                signature_key: signer.public().to_vec(),
                label: label.to_owned(),
            },
            "link request",
        )?;
        Ok(Self {
            secret,
            signer,
            device,
            request,
        })
    }

    /// The six digits the existing device must show too.
    #[must_use]
    pub fn match_code(&self) -> String {
        self.secret.match_code(&self.request)
    }

    /// Leaves the sealed request in the slot.
    ///
    /// # Errors
    ///
    /// A crypto error.
    pub fn request(&self) -> Result<Request> {
        Ok(self
            .secret
            .put("request", self.secret.seal(Kind::Request, &self.request)?))
    }

    /// Asks the service what is in the slot.
    #[must_use]
    pub fn check(&self) -> Request {
        self.secret.get()
    }

    /// Reads the answer to [`Joining::check`]: what the existing device
    /// sent, once it approved. The certificate must be for this very device
    /// and key.
    ///
    /// # Errors
    ///
    /// [`CoreError::Service`] (`unknown_link` once the slot expired or was
    /// taken back), [`CoreError::Unauthenticated`], or
    /// [`CoreError::Malformed`].
    pub fn linked(&self, status: u16, body: &[u8]) -> Result<Option<Linked>> {
        let stored = LinkSecret::slot(status, body)?.ok_or(CoreError::Service {
            status,
            code: "unknown_link".to_owned(),
        })?;
        let Some(sealed) = stored.answer else {
            return Ok(None);
        };
        let answer: AnswerBody =
            from_cbor(&self.secret.open(Kind::Answer, &sealed)?, "link answer")?;
        let certificate = DeviceCertificate::decode(&answer.certificate)?;
        certificate.verify()?;
        if certificate.device() != self.device
            || certificate.signature_key() != self.signer.public()
        {
            return Err(CoreError::Unauthenticated(
                "the certificate is for another device",
            ));
        }
        Ok(Some(Linked {
            certificate,
            username: service::normalize_username(&answer.username)?,
            contacts: answer.contacts,
            introduced: answer.introduced,
            labels: answer
                .labels
                .into_iter()
                .filter(|(_, label)| label.chars().count() <= MAX_LABEL)
                .collect(),
        }))
    }

    /// Clears the slot once the link is done or abandoned.
    #[must_use]
    pub fn close(&self) -> Request {
        Request {
            method: "DELETE",
            path: self.secret.path(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// Reads the service's answer to a request or answer put into the slot.
///
/// # Errors
///
/// [`CoreError::Service`]: `link_taken` when another device got there
/// first, `unknown_link` when the slot expired.
pub fn stored(status: u16, body: &[u8]) -> Result<()> {
    service::expect_success(status, body, &[200, 201])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_twenty_characters_in_four_groups_and_never_repeat() {
        let (a, b) = (Offer::new().expect("offer"), Offer::new().expect("offer"));
        let code = a.code();
        assert_eq!(code.len(), CODE_LEN + 3);
        assert!(code.split('-').all(|group| group.len() == 5));
        assert!(
            code.bytes()
                .all(|byte| byte == b'-' || ALPHABET.contains(&byte))
        );
        assert_ne!(code, b.code());
        assert_ne!(a.check().path, b.check().path);
    }

    #[test]
    fn a_code_reads_however_it_is_typed() {
        let offer = Offer::new().expect("offer");
        let typed = format!(" {} ", offer.code().to_uppercase().replace('-', " "));
        let joining = Joining::new(&typed, "Phone").expect("joining");
        assert_eq!(joining.check().path, offer.check().path);
        for wrong in ["", "abcde", &offer.code()[1..], "0bcde-fghij-klmno-pqrst"] {
            assert!(matches!(
                Joining::new(wrong, "Phone"),
                Err(CoreError::Malformed { .. })
            ));
        }
        assert!(Joining::new(&offer.code(), &"x".repeat(MAX_LABEL + 1)).is_err());
    }

    #[test]
    fn sealed_messages_open_only_as_what_they_are() {
        let secret = LinkSecret::generate().expect("secret");
        let sealed = secret.seal(Kind::Request, b"hello").expect("seal");
        assert_eq!(&sealed[..4], b"ZL\x00\x01");
        assert_eq!(
            secret
                .open(Kind::Request, &sealed)
                .expect("open")
                .as_slice(),
            b"hello"
        );
        assert!(secret.open(Kind::Answer, &sealed).is_err());

        let mut tampered = sealed.clone();
        *tampered.last_mut().expect("byte") ^= 1;
        assert!(matches!(
            secret.open(Kind::Request, &tampered),
            Err(CoreError::Unauthenticated(_))
        ));
        let other = LinkSecret::generate().expect("secret");
        assert!(other.open(Kind::Request, &sealed).is_err());
        assert!(secret.open(Kind::Request, &sealed[..20]).is_err());
    }

    #[test]
    fn match_codes_are_six_digits_and_follow_the_request() {
        let secret = LinkSecret::generate().expect("secret");
        let code = secret.match_code(b"one request");
        assert_eq!(code.len(), 7);
        assert!(code.chars().enumerate().all(|(at, c)| if at == 3 {
            c == ' '
        } else {
            c.is_ascii_digit()
        }));
        assert_eq!(code, secret.match_code(b"one request"));
        assert_ne!(code, secret.match_code(b"another request"));
    }
}
