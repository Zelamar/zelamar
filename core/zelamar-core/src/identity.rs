//! Account and device identity.
//!
//! Every account owns a long-term Ed25519 **account key**, the root of trust.
//! Every device (phone, PC) has its own MLS signature key. The account key
//! signs a [`DeviceCertificate`] binding that device key to the account.
//! Peers verify the certificate before they accept a device into a chat, so
//! the server can never slip in a key of its own (the prototype's silent
//! key-substitution flaw).

use openmls::prelude::SignatureScheme;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{crypto::OpenMlsCrypto, random::OpenMlsRand, signatures::Signer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use crate::error::{CoreError, Result};

/// Signature scheme of account keys.
pub const ACCOUNT_SIGNATURE_SCHEME: SignatureScheme = SignatureScheme::ED25519;

/// Domain separation for device certificates: a signature made for this
/// purpose can never be replayed as a signature over anything else.
const DEVICE_CERTIFICATE_CONTEXT: &[u8] = b"zelamar/device-certificate/v0";

/// Wire version of [`DeviceCertificate::encode`].
const DEVICE_CERTIFICATE_VERSION: u8 = 0;

const ED25519_PUBLIC_KEY_LEN: usize = 32;
const ED25519_SIGNATURE_LEN: usize = 64;

/// Public identity of an account: its Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AccountId(#[serde(with = "serde_bytes")] [u8; ED25519_PUBLIC_KEY_LEN]);

impl AccountId {
    /// The account with public key `bytes`. Whether such an account exists,
    /// or signs anything, is checked wherever its signatures are verified.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; ED25519_PUBLIC_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw public key bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; ED25519_PUBLIC_KEY_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for AccountId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AccountId({})", short_hex(&self.0))
    }
}

/// Random identifier of one device of an account.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(#[serde(with = "serde_bytes")] [u8; 16]);

impl DeviceId {
    /// Draws a fresh random device id.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] if the system random generator fails.
    pub fn random() -> Result<Self> {
        RustCrypto::default()
            .random_array()
            .map(Self)
            .map_err(|e| CoreError::crypto("drawing a device id", e))
    }

    /// The raw id bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl std::fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceId({})", short_hex(&self.0))
    }
}

/// The secret account key. Holds the root of trust: never leaves the device
/// unencrypted, never logged.
pub struct AccountKey {
    keys: SignatureKeyPair,
    id: AccountId,
}

impl AccountKey {
    /// Generates a new account key.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] if key generation fails.
    pub fn generate() -> Result<Self> {
        let keys = SignatureKeyPair::new(ACCOUNT_SIGNATURE_SCHEME)
            .map_err(|e| CoreError::crypto("generating the account key", e))?;
        let id =
            AccountId(keys.public().try_into().map_err(|_| {
                CoreError::malformed("account key", "unexpected public key length")
            })?);
        Ok(Self { keys, id })
    }

    /// The public identity of this account.
    #[must_use]
    pub const fn id(&self) -> AccountId {
        self.id
    }

    /// Serializes the secret key for the encrypted store. The buffer is wiped
    /// when dropped.
    pub(crate) fn to_secret_bytes(&self) -> Result<zeroize::Zeroizing<Vec<u8>>> {
        let mut out = zeroize::Zeroizing::new(Vec::new());
        ciborium::into_writer(&self.keys, &mut *out).map_err(|e| CoreError::Encoding {
            what: "account key",
            source: e.into(),
        })?;
        Ok(out)
    }

    /// Restores a key written by [`Self::to_secret_bytes`].
    pub(crate) fn from_secret_bytes(bytes: &[u8]) -> Result<Self> {
        let keys: SignatureKeyPair = ciborium::from_reader(bytes)
            .map_err(|_| CoreError::malformed("account key", "invalid encoding"))?;
        let id =
            AccountId(keys.public().try_into().map_err(|_| {
                CoreError::malformed("account key", "unexpected public key length")
            })?);
        Ok(Self { keys, id })
    }

    /// Signs a Zelamar protocol payload with the account key. Every such
    /// payload starts with its own context string (for example
    /// `zelamar/claim/v0`), so a signature for one purpose never verifies as
    /// another.
    pub(crate) fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        self.keys
            .sign(payload)
            .map_err(|e| CoreError::crypto("signing with the account key", format!("{e:?}")))
    }

    /// Issues a certificate binding `device_signature_key` to this account.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] if signing fails.
    pub fn certify_device(
        &self,
        device: DeviceId,
        device_signature_key: &[u8],
    ) -> Result<DeviceCertificate> {
        let payload = DeviceCertificate::signed_payload(self.id, device, device_signature_key);
        let signature = self
            .keys
            .sign(&payload)
            .map_err(|e| CoreError::crypto("signing a device certificate", format!("{e:?}")))?;
        Ok(DeviceCertificate {
            account: self.id,
            device,
            signature_key: device_signature_key.to_vec(),
            signature,
        })
    }
}

impl std::fmt::Debug for AccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Statement "device `device` with MLS signature key `signature_key` belongs
/// to account `account`", signed by the account key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCertificate {
    account: AccountId,
    device: DeviceId,
    signature_key: Vec<u8>,
    signature: Vec<u8>,
}

impl DeviceCertificate {
    /// The account the device belongs to.
    #[must_use]
    pub const fn account(&self) -> AccountId {
        self.account
    }

    /// The certified device.
    #[must_use]
    pub const fn device(&self) -> DeviceId {
        self.device
    }

    /// The device's MLS signature public key.
    #[must_use]
    pub fn signature_key(&self) -> &[u8] {
        &self.signature_key
    }

    fn signed_payload(account: AccountId, device: DeviceId, signature_key: &[u8]) -> Vec<u8> {
        let mut payload = Vec::with_capacity(
            DEVICE_CERTIFICATE_CONTEXT.len()
                + ED25519_PUBLIC_KEY_LEN
                + 16
                + 2
                + signature_key.len(),
        );
        payload.extend_from_slice(DEVICE_CERTIFICATE_CONTEXT);
        payload.extend_from_slice(&account.0);
        payload.extend_from_slice(&device.0);
        push_with_u16_len(&mut payload, signature_key);
        payload
    }

    /// Checks the account's signature over this certificate.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] if the signature does not verify.
    pub fn verify(&self) -> Result<()> {
        let payload = Self::signed_payload(self.account, self.device, &self.signature_key);
        RustCrypto::default()
            .verify_signature(
                ACCOUNT_SIGNATURE_SCHEME,
                &payload,
                &self.account.0,
                &self.signature,
            )
            .map_err(|_| CoreError::Unauthenticated("device certificate signature is invalid"))
    }

    /// Serializes the certificate (version 0 layout, see `protocol/`).
    ///
    /// # Panics
    ///
    /// Never in practice: key and signature lengths are bounded by the
    /// signature scheme and always fit the 16-bit length prefixes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + ED25519_PUBLIC_KEY_LEN + 16 + 4 + 96);
        out.push(DEVICE_CERTIFICATE_VERSION);
        out.extend_from_slice(&self.account.0);
        out.extend_from_slice(&self.device.0);
        push_with_u16_len(&mut out, &self.signature_key);
        push_with_u16_len(&mut out, &self.signature);
        out
    }

    /// Parses a certificate produced by [`encode`](Self::encode). Does **not**
    /// verify it; call [`verify`](Self::verify).
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] or [`CoreError::UnsupportedVersion`] for
    /// anything but a well-formed version 0 certificate.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        const WHAT: &str = "device certificate";
        let mut reader = Reader::new(bytes, WHAT);
        let version = reader.u8()?;
        if version != DEVICE_CERTIFICATE_VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: WHAT,
                version: version.into(),
            });
        }
        let account = AccountId(reader.array()?);
        let device = DeviceId(reader.array()?);
        let signature_key = reader.bytes_with_u16_len()?.to_vec();
        let signature = reader.bytes_with_u16_len()?.to_vec();
        reader.finish()?;
        if signature_key.len() != ED25519_PUBLIC_KEY_LEN {
            return Err(CoreError::malformed(
                WHAT,
                "device key has the wrong length",
            ));
        }
        if signature.len() != ED25519_SIGNATURE_LEN {
            return Err(CoreError::malformed(WHAT, "signature has the wrong length"));
        }
        Ok(Self {
            account,
            device,
            signature_key,
            signature,
        })
    }
}

/// Domain separation for safety codes.
const SAFETY_CODE_CONTEXT: &[u8] = b"zelamar/safety-code/v0";

/// SHA-512 rounds per fingerprint. Makes grinding for an account key whose
/// code collides with a victim's expensive (same order as Signal).
const SAFETY_CODE_ROUNDS: usize = 5200;

/// A 60-digit code two people compare in person (or by QR) to confirm that
/// no one sits between them. Both sides compute the same code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyCode([u32; 12]);

impl SafetyCode {
    /// The code for the conversation between accounts `a` and `b`. The order
    /// of the arguments does not matter.
    #[must_use]
    pub fn between(a: &AccountId, b: &AccountId) -> Self {
        let ordered = if a.0 <= b.0 { [a, b] } else { [b, a] };
        let mut groups = [0u32; 12];
        for (half, account) in ordered.into_iter().enumerate() {
            let digest = fingerprint(account);
            let (chunks, _) = digest.as_chunks::<5>();
            for (i, chunk) in chunks.iter().take(6).enumerate() {
                let value = chunk
                    .iter()
                    .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
                // The remainder is always below 100 000, so it fits a u32.
                groups[half * 6 + i] = u32::try_from(value % 100_000).unwrap_or(0);
            }
        }
        Self(groups)
    }

    /// The twelve five-digit groups, as shown to people.
    #[must_use]
    pub const fn groups(&self) -> [u32; 12] {
        self.0
    }
}

impl std::fmt::Display for SafetyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, group) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{group:05}")?;
        }
        Ok(())
    }
}

/// Iterated SHA-512 over one account key.
fn fingerprint(account: &AccountId) -> [u8; 64] {
    let mut digest: [u8; 64] = Sha512::new()
        .chain_update(SAFETY_CODE_CONTEXT)
        .chain_update(account.0)
        .finalize()
        .into();
    for _ in 0..SAFETY_CODE_ROUNDS {
        digest = Sha512::new()
            .chain_update(digest)
            .chain_update(account.0)
            .finalize()
            .into();
    }
    digest
}

fn push_with_u16_len(out: &mut Vec<u8>, data: &[u8]) {
    let len = u16::try_from(data.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&data[..usize::from(len)]);
}

fn short_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().take(6).fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }) + "…"
}

/// Minimal bounds-checked reader for the core's fixed binary layouts.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    what: &'static str,
}

impl<'a> Reader<'a> {
    pub(crate) const fn new(bytes: &'a [u8], what: &'static str) -> Self {
        Self { bytes, what }
    }

    pub(crate) const fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < n {
            return Err(CoreError::malformed(self.what, "unexpected end of data"));
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let slice = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    pub(crate) fn bytes_with_u16_len(&mut self) -> Result<&'a [u8]> {
        let len = u16::from_be_bytes(self.array()?);
        self.take(usize::from(len))
    }

    pub(crate) fn bytes_with_u32_len(&mut self, limit: usize) -> Result<&'a [u8]> {
        let len = usize::try_from(u32::from_be_bytes(self.array()?))
            .map_err(|_| CoreError::malformed(self.what, "length does not fit in memory"))?;
        if len > limit {
            return Err(CoreError::TooLarge {
                what: self.what,
                size: len,
                limit,
            });
        }
        self.take(len)
    }

    pub(crate) const fn finish(self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(CoreError::malformed(self.what, "trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate() -> (AccountKey, DeviceCertificate) {
        let account = AccountKey::generate().expect("account key");
        let device = DeviceId::random().expect("device id");
        let cert = account
            .certify_device(device, &[7u8; 32])
            .expect("certificate");
        (account, cert)
    }

    #[test]
    fn certificate_round_trips_and_verifies() {
        let (account, cert) = certificate();
        let decoded = DeviceCertificate::decode(&cert.encode()).expect("decode");
        assert_eq!(decoded, cert);
        assert_eq!(decoded.account(), account.id());
        decoded.verify().expect("valid signature");
    }

    #[test]
    fn tampered_device_key_is_rejected() {
        let (_, cert) = certificate();
        let mut forged = cert;
        forged.signature_key[0] ^= 1;
        assert!(matches!(
            forged.verify(),
            Err(CoreError::Unauthenticated(_))
        ));
    }

    #[test]
    fn certificate_from_another_account_is_rejected() {
        let (_, cert) = certificate();
        let impostor = AccountKey::generate().expect("account key");
        let mut forged = cert;
        forged.account = impostor.id();
        assert!(matches!(
            forged.verify(),
            Err(CoreError::Unauthenticated(_))
        ));
    }

    #[test]
    fn decode_rejects_unknown_version_and_trailing_bytes() {
        let (_, cert) = certificate();
        let mut bytes = cert.encode();
        bytes[0] = 9;
        assert!(matches!(
            DeviceCertificate::decode(&bytes),
            Err(CoreError::UnsupportedVersion { version: 9, .. })
        ));

        let mut bytes = cert.encode();
        bytes.push(0);
        assert!(matches!(
            DeviceCertificate::decode(&bytes),
            Err(CoreError::Malformed { .. })
        ));
    }

    #[test]
    fn safety_codes_agree_on_both_sides() {
        let alice = AccountKey::generate().expect("account key").id();
        let bob = AccountKey::generate().expect("account key").id();
        assert_eq!(
            SafetyCode::between(&alice, &bob),
            SafetyCode::between(&bob, &alice)
        );
    }

    #[test]
    fn safety_codes_differ_between_pairs_and_have_sixty_digits() {
        let alice = AccountKey::generate().expect("account key").id();
        let bob = AccountKey::generate().expect("account key").id();
        let mallory = AccountKey::generate().expect("account key").id();
        let genuine = SafetyCode::between(&alice, &bob);
        assert_ne!(genuine, SafetyCode::between(&alice, &mallory));

        let shown = genuine.to_string();
        let groups: Vec<&str> = shown.split(' ').collect();
        assert_eq!(groups.len(), 12);
        assert!(
            groups
                .iter()
                .all(|g| g.len() == 5 && g.bytes().all(|b| b.is_ascii_digit()))
        );
    }

    #[test]
    fn safety_codes_are_stable() {
        let a = AccountId([1; 32]);
        let b = AccountId([2; 32]);
        assert_eq!(SafetyCode::between(&a, &b), SafetyCode::between(&a, &b));
        assert_ne!(SafetyCode::between(&a, &b).groups(), [0; 12]);
    }

    #[test]
    fn debug_output_never_contains_key_material() {
        let account = AccountKey::generate().expect("account key");
        let printed = format!("{account:?}");
        assert!(printed.starts_with("AccountKey { id: AccountId("));
        assert!(printed.ends_with(".. }"));
    }
}
