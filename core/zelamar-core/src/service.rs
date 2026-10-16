//! Talking to the delivery service without doing any I/O (ADR-0012).
//!
//! The core builds every request (signed, with proofs of work) and checks
//! every answer. The app only moves bytes over HTTPS and WebSocket, so all
//! security-relevant logic lives here, once. Formats:
//! `protocol/directory.md` and `protocol/delivery.md`.
//!
//! A session looks like this:
//! 1. [`Device::claim`] a username; [`Device::claimed`] reads the answer.
//! 2. [`Device::register`] the device with its mailbox and key packages;
//!    [`key_package_count`] reads the answer.
//! 3. [`lookup`] a contact; [`directory_entry`] reads the answer; then
//!    [`Device::start_chat`].
//! 4. For each of [`Device::unconfirmed_contacts`], send [`key_lookup`] and
//!    pass [`directory_entry`] to [`Device::confirm_contact`].
//! 5. For every item in [`Device::outbox`], send [`delivery`] and, when
//!    [`delivered`] accepts the answer, call [`Device::confirm_sent`].
//! 6. Connect to [`Device::mailbox_access`] over WebSocket. Pass every
//!    binary message to [`Device::open_envelope`], then send
//!    [`acknowledgement`] for it unless [`Opened::keep`] says to keep it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::device::{Device, Outgoing, Received};
use crate::envelope::{Envelope, EnvelopeId, MAX_PAYLOAD};
use crate::error::{CoreError, Result};
use crate::identity::{AccountId, AccountKey, DeviceCertificate, DeviceId};

/// Proof-of-work difficulty of the production service, in leading zero bits.
pub const CLAIM_WORK_BITS: u8 = 21;

/// One-time key packages published when a device registers.
pub const KEY_PACKAGES_AT_REGISTRATION: usize = 50;

/// Below this many stored key packages, a device should replenish.
pub const KEY_PACKAGES_LOW_WATER: u64 = 10;

/// Most recipients one delivery frame can name.
pub const MAX_RECIPIENTS: usize = 255;

const CLAIM_CONTEXT: &[u8] = b"zelamar/claim/v0";
const WORK_CONTEXT: &[u8] = b"zelamar/work/v0";
const REQUEST_CONTEXT: &[u8] = b"zelamar/request/v0";
const DELIVERY_MAGIC: &[u8] = b"ZD";
const COMMIT_MAGIC: &[u8] = b"ZC";
const DELIVERY_VERSION: u8 = 0;
const ACK: u8 = 0x01;

/// An HTTP request for the delivery service, ready to send as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The HTTP method.
    pub method: &'static str,
    /// The path, relative to the service's base URL (`/v0/...`).
    pub path: String,
    /// Headers to send, besides the transport's own.
    pub headers: Vec<(&'static str, String)>,
    /// The body.
    pub body: Vec<u8>,
}

/// Where and how a device reads its mailbox: a WebSocket to `path` with an
/// `Authorization: <authorization>` header.
#[derive(Clone, PartialEq, Eq)]
pub struct MailboxAccess {
    /// The path, relative to the service's base URL.
    pub path: String,
    /// The `Authorization` header value; holds the mailbox secret.
    pub authorization: String,
}

impl std::fmt::Debug for MailboxAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailboxAccess")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// What the directory says about a username.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// The username, normalized.
    pub username: String,
    /// The account that holds it.
    pub account: AccountId,
    /// One key package per device, certified by `account`. Empty for a
    /// [`key_lookup`].
    pub key_packages: Vec<Vec<u8>>,
}

/// One envelope taken from the mailbox.
#[derive(Debug)]
pub struct Opened {
    /// The envelope id, to acknowledge.
    pub id: EnvelopeId,
    /// What it turned out to be.
    pub result: Result<Received>,
}

impl Opened {
    /// Whether to leave the envelope in the mailbox for a later attempt
    /// (a transient failure). Otherwise it is acknowledged: delivered, or
    /// refused for good, so it never blocks the queue.
    #[must_use]
    pub fn keep(&self) -> bool {
        self.result.as_ref().is_err_and(CoreError::is_transient)
    }
}

/// `input` lowercased if it is a well-formed username: 3–32 ASCII letters,
/// digits and single underscores, starting with a letter, not ending with an
/// underscore.
///
/// # Errors
///
/// [`CoreError::Malformed`] otherwise.
pub fn normalize_username(input: &str) -> Result<String> {
    let well_formed = (3..=32).contains(&input.len())
        && input
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && input
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        && !input.contains("__")
        && !input.ends_with('_');
    if well_formed {
        Ok(input.to_ascii_lowercase())
    } else {
        Err(CoreError::malformed(
            "username",
            "use 3-32 letters, digits or single underscores, starting with a letter",
        ))
    }
}

/// Claims `username` for `account`, with a proof of work of `work_bits`
/// ([`CLAIM_WORK_BITS`] for the real service).
///
/// # Errors
///
/// [`CoreError::Malformed`] for a bad username, or a crypto error.
pub fn claim(
    account: &AccountKey,
    username: &str,
    invite: &str,
    now_ms: u64,
    work_bits: u8,
) -> Result<Request> {
    let username = normalize_username(username)?;
    let name = name_field(&username);
    let work = solve_work(&account.id(), &name, work_bits);
    let signature = account.sign(&[CLAIM_CONTEXT, &name, &now_ms.to_be_bytes(), &work].concat())?;
    json_request(
        "POST",
        "/v0/accounts".to_owned(),
        &json!({
            "username": username,
            "account": b64(account.id().as_bytes()),
            "invite": invite,
            "work": b64(&work),
            "timestamp": now_ms,
            "signature": b64(&signature),
        }),
    )
}

/// Reads the answer to [`claim`]: the username now held.
///
/// # Errors
///
/// [`CoreError::Service`] with the service's code (`username_taken`,
/// `invalid_invite`, ...), or [`CoreError::Malformed`].
pub fn claimed(status: u16, body: &[u8]) -> Result<String> {
    #[derive(Deserialize)]
    struct Claimed {
        username: String,
    }
    expect_success(status, body, &[200, 201])?;
    parse::<Claimed>(body).map(|claimed| claimed.username)
}

/// Looks up `username` in the directory.
///
/// # Errors
///
/// [`CoreError::Malformed`] for a bad username.
pub fn lookup(username: &str) -> Result<Request> {
    Ok(Request {
        method: "GET",
        path: format!("/v0/accounts/{}", normalize_username(username)?),
        headers: Vec::new(),
        body: Vec::new(),
    })
}

/// Asks the directory which account holds `username`, without taking any
/// key package: for confirming a name someone claims.
///
/// # Errors
///
/// [`CoreError::Malformed`] for a bad username.
pub fn key_lookup(username: &str) -> Result<Request> {
    Ok(Request {
        method: "GET",
        path: format!("/v0/accounts/{}/key", normalize_username(username)?),
        headers: Vec::new(),
        body: Vec::new(),
    })
}

/// Asks the directory which devices `username` has registered, without
/// taking any key package (ADR-0014).
///
/// # Errors
///
/// [`CoreError::Malformed`] for a bad username.
pub fn devices(username: &str) -> Result<Request> {
    Ok(Request {
        method: "GET",
        path: format!("/v0/accounts/{}/devices", normalize_username(username)?),
        headers: Vec::new(),
        body: Vec::new(),
    })
}

/// Reads the answer to [`devices`] for `requested`: the account and its
/// devices' certificates, each verified under it.
///
/// # Errors
///
/// As [`directory_entry`].
pub fn device_list(
    requested: &str,
    status: u16,
    body: &[u8],
) -> Result<(AccountId, Vec<DeviceCertificate>)> {
    #[derive(Deserialize)]
    struct Found {
        username: String,
        account: String,
        devices: Vec<Listed>,
    }
    #[derive(Deserialize)]
    struct Listed {
        certificate: String,
    }

    expect_success(status, body, &[200])?;
    let found: Found = parse(body)?;
    if found.username != normalize_username(requested)? {
        return Err(CoreError::malformed(
            "directory answer",
            "about another username",
        ));
    }
    let account = account_of(&found.account)?;
    let certificates = found
        .devices
        .iter()
        .map(|device| certified(&device.certificate, account))
        .collect::<Result<Vec<_>>>()?;
    Ok((account, certificates))
}

/// Takes one key package of one device of `username`, for adding that
/// device to a chat (ADR-0014). The answer reads with [`directory_entry`].
///
/// # Errors
///
/// [`CoreError::Malformed`] for a bad username.
pub fn device_lookup(username: &str, device: DeviceId) -> Result<Request> {
    Ok(Request {
        method: "GET",
        path: format!(
            "/v0/accounts/{}/devices/{}",
            normalize_username(username)?,
            b64(device.as_bytes())
        ),
        headers: Vec::new(),
        body: Vec::new(),
    })
}

/// An account key as the directory writes it.
fn account_of(text: &str) -> Result<AccountId> {
    Ok(AccountId::from_bytes(unb64(text)?.try_into().map_err(
        |_| CoreError::malformed("contact", "account is not 32 bytes"),
    )?))
}

/// A device certificate the directory listed for `account`, verified.
fn certified(text: &str, account: AccountId) -> Result<DeviceCertificate> {
    let certificate = DeviceCertificate::decode(&unb64(text)?)?;
    certificate.verify()?;
    if certificate.account() != account {
        return Err(CoreError::Unauthenticated(
            "the directory listed a device of another account",
        ));
    }
    Ok(certificate)
}

/// Reads the answer to [`lookup`] or [`key_lookup`] for `requested`. The
/// answer must be about that very username, and every certificate must
/// verify under the account key; [`Device::start_chat`] verifies the rest.
///
/// # Errors
///
/// [`CoreError::Service`] (`unknown_account`, ...),
/// [`CoreError::Unauthenticated`] for a device the account did not certify,
/// or [`CoreError::Malformed`].
pub fn directory_entry(requested: &str, status: u16, body: &[u8]) -> Result<DirectoryEntry> {
    #[derive(Deserialize)]
    struct Found {
        username: String,
        account: String,
        #[serde(default)]
        devices: Vec<FoundDevice>,
    }
    #[derive(Deserialize)]
    struct FoundDevice {
        certificate: String,
        key_package: String,
    }

    expect_success(status, body, &[200])?;
    let found: Found = parse(body)?;
    if found.username != normalize_username(requested)? {
        return Err(CoreError::malformed(
            "directory answer",
            "about another username",
        ));
    }
    let account = account_of(&found.account)?;
    let key_packages = found
        .devices
        .iter()
        .map(|device| {
            certified(&device.certificate, account)?;
            unb64(&device.key_package)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(DirectoryEntry {
        username: found.username,
        account,
        key_packages,
    })
}

/// The delivery frame for `outgoing`.
///
/// # Errors
///
/// [`CoreError::Malformed`] without recipients or with more than
/// [`MAX_RECIPIENTS`]; [`CoreError::TooLarge`] for an oversized payload.
pub fn delivery(outgoing: &Outgoing) -> Result<Request> {
    if let Some(ticket) = &outgoing.commit {
        let mut body = Vec::new();
        body.extend_from_slice(COMMIT_MAGIC);
        body.push(DELIVERY_VERSION);
        body.extend_from_slice(&ticket.epoch.to_be_bytes());
        body.extend_from_slice(&delivery_frame(outgoing, true)?);
        if let Some(welcome) = &ticket.welcome {
            body.extend_from_slice(&delivery_frame(welcome, false)?);
        }
        return Ok(Request {
            method: "POST",
            path: format!("/v0/groups/{}/commits", b64(&ticket.group)),
            headers: vec![("content-type", "application/octet-stream".to_owned())],
            body,
        });
    }
    Ok(Request {
        method: "POST",
        path: "/v0/deliver".to_owned(),
        headers: vec![("content-type", "application/octet-stream".to_owned())],
        body: delivery_frame(outgoing, false)?,
    })
}

/// One delivery frame; a commit's may have no recipients.
fn delivery_frame(outgoing: &Outgoing, allow_empty: bool) -> Result<Vec<u8>> {
    let count = u8::try_from(outgoing.recipients.len())
        .ok()
        .filter(|&count| count > 0 || allow_empty)
        .ok_or(CoreError::malformed(
            "delivery",
            "needs 1 to 255 recipients",
        ))?;
    let length = u32::try_from(outgoing.payload.len())
        .ok()
        .filter(|_| outgoing.payload.len() <= MAX_PAYLOAD)
        .ok_or(CoreError::TooLarge {
            what: "payload",
            size: outgoing.payload.len(),
            limit: MAX_PAYLOAD,
        })?;
    let mut body = Vec::with_capacity(24 + 32 * usize::from(count) + outgoing.payload.len());
    body.extend_from_slice(DELIVERY_MAGIC);
    body.push(DELIVERY_VERSION);
    body.extend_from_slice(&outgoing.id.0);
    body.push(count);
    for recipient in &outgoing.recipients {
        body.extend_from_slice(&recipient.0);
    }
    body.extend_from_slice(&length.to_be_bytes());
    body.extend_from_slice(&outgoing.payload);
    Ok(body)
}

/// The sequencer's answer to a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitAnswer {
    /// The commit won; `epoch` is the chat's new epoch.
    Accepted {
        /// The new epoch.
        epoch: u64,
    },
    /// Another commit won this epoch; `epoch` is the chat's current one.
    Refused {
        /// The current epoch.
        epoch: u64,
    },
}

/// Reads the sequencer's answer to a commit request.
///
/// # Errors
///
/// [`CoreError::Service`] for anything but acceptance or `epoch_mismatch`,
/// or [`CoreError::Malformed`].
pub fn commit_answer(status: u16, body: &[u8]) -> Result<CommitAnswer> {
    #[derive(Deserialize)]
    struct Answer {
        epoch: u64,
        error: Option<String>,
    }
    if status == 409
        && let Ok(answer) = serde_json::from_slice::<Answer>(body)
        && answer.error.as_deref() == Some("epoch_mismatch")
    {
        return Ok(CommitAnswer::Refused {
            epoch: answer.epoch,
        });
    }
    expect_success(status, body, &[202])?;
    parse::<Answer>(body).map(|answer| CommitAnswer::Accepted {
        epoch: answer.epoch,
    })
}

/// Reads the answer to [`delivery`]: how many recipients' mailboxes took
/// it. On success the item is out of the device's hands; confirm it.
///
/// # Errors
///
/// [`CoreError::Service`] (`rate_limited`, ...) or [`CoreError::Malformed`].
pub fn delivered(status: u16, body: &[u8]) -> Result<u32> {
    #[derive(Deserialize)]
    struct Delivered {
        delivered: u32,
    }
    expect_success(status, body, &[202])?;
    parse::<Delivered>(body).map(|answer| answer.delivered)
}

/// The acknowledgement frame for envelopes taken from the mailbox.
#[must_use]
pub fn acknowledgement(ids: &[EnvelopeId]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + 16 * ids.len());
    frame.push(ACK);
    for id in ids {
        frame.extend_from_slice(&id.0);
    }
    frame
}

/// Reads the answer to [`Device::register`] or
/// [`Device::replenish_key_packages`]: how many key packages the service
/// holds for the device.
///
/// # Errors
///
/// [`CoreError::Service`] or [`CoreError::Malformed`].
pub fn key_package_count(status: u16, body: &[u8]) -> Result<u64> {
    #[derive(Deserialize)]
    struct Stored {
        key_packages: u64,
    }
    expect_success(status, body, &[200])?;
    parse::<Stored>(body).map(|stored| stored.key_packages)
}

impl Device {
    /// Claims `username` for this device's account (see [`claim`]).
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] on a device without the account key,
    /// or as [`claim`].
    pub fn claim(
        &self,
        username: &str,
        invite: &str,
        now_ms: u64,
        work_bits: u8,
    ) -> Result<Request> {
        let account = self.account_key().ok_or(CoreError::Unauthenticated(
            "only the device holding the account key can claim a username",
        ))?;
        claim(account, username, invite, now_ms, work_bits)
    }

    /// Reads the answer to [`Device::claim`] and remembers the username.
    ///
    /// # Errors
    ///
    /// As [`claimed`], or a storage error.
    pub fn claimed(&mut self, status: u16, body: &[u8]) -> Result<String> {
        let username = claimed(status, body)?;
        self.set_username(&username)?;
        Ok(username)
    }

    /// Registers this device under `username`: its certificate, mailbox
    /// and [`KEY_PACKAGES_AT_REGISTRATION`] key packages plus a last-resort
    /// one. Repeating it is harmless.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for a bad username, or a crypto, MLS or
    /// storage error.
    pub fn register(&mut self, username: &str, now_ms: u64) -> Result<Request> {
        let username = normalize_username(username)?;
        let fresh = self.key_packages(KEY_PACKAGES_AT_REGISTRATION, true)?;
        let body = json!({
            "mailbox": b64(&self.mailbox().0),
            "mailbox_secret_hash": b64(&Sha256::digest(self.mailbox_secret())),
            "key_packages": fresh.one_time.iter().map(|kp| b64(kp)).collect::<Vec<_>>(),
            "last_resort": fresh.last_resort.as_deref().map(b64),
        });
        self.signed("PUT", self.device_path(&username), &body, now_ms)
    }

    /// Publishes `count` more one-time key packages under `username`.
    ///
    /// # Errors
    ///
    /// As [`Device::register`].
    pub fn replenish_key_packages(
        &mut self,
        username: &str,
        count: usize,
        now_ms: u64,
    ) -> Result<Request> {
        let username = normalize_username(username)?;
        let fresh = self.key_packages(count, false)?;
        let body = json!({
            "key_packages": fresh.one_time.iter().map(|kp| b64(kp)).collect::<Vec<_>>(),
        });
        self.signed(
            "POST",
            format!("{}/key-packages", self.device_path(&username)),
            &body,
            now_ms,
        )
    }

    /// How to read this device's mailbox.
    #[must_use]
    pub fn mailbox_access(&self) -> MailboxAccess {
        MailboxAccess {
            path: format!("/v0/mailboxes/{}", b64(&self.mailbox().0)),
            authorization: format!("Bearer {}", b64(self.mailbox_secret())),
        }
    }

    /// Opens one envelope from the mailbox: a message, a chat update or an
    /// invitation into a chat.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] if the bytes are not an envelope at all; the
    /// mailbox service never stores such a thing, so this means a broken
    /// transport. Everything that goes wrong after that is in
    /// [`Opened::result`].
    pub fn open_envelope(&mut self, bytes: &[u8]) -> Result<Opened> {
        let envelope = Envelope::decode(bytes)?;
        let result = if envelope.to == self.mailbox() {
            self.receive(&envelope.payload)
        } else {
            Err(CoreError::malformed(
                "envelope",
                "addressed to another mailbox",
            ))
        };
        Ok(Opened {
            id: envelope.id,
            result,
        })
    }

    fn device_path(&self, username: &str) -> String {
        format!(
            "/v0/accounts/{username}/devices/{}",
            b64(self.id().as_bytes())
        )
    }

    /// A request signed by this device (`protocol/directory.md`).
    fn signed(
        &self,
        method: &'static str,
        path: String,
        body: &serde_json::Value,
        now_ms: u64,
    ) -> Result<Request> {
        let mut request = json_request(method, path, body)?;
        let payload = [
            REQUEST_CONTEXT,
            method.as_bytes(),
            &[0],
            request.path.as_bytes(),
            &[0],
            &now_ms.to_be_bytes(),
            &Sha256::digest(&request.body),
        ]
        .concat();
        let signature = self.sign(&payload)?;
        request.headers.extend([
            ("zelamar-certificate", b64(&self.certificate().encode())),
            ("zelamar-timestamp", now_ms.to_string()),
            ("zelamar-signature", b64(&signature)),
        ]);
        Ok(request)
    }
}

/// `u8 length || username`, as both the claim and the proof of work use it.
fn name_field(username: &str) -> Vec<u8> {
    // A normalized username is at most 32 bytes.
    let length = u8::try_from(username.len()).unwrap_or(u8::MAX);
    [&[length][..], username.as_bytes()].concat()
}

/// The first 8-byte counter whose proof has `bits` leading zero bits.
fn solve_work(account: &AccountId, name: &[u8], bits: u8) -> [u8; 8] {
    let mut prefix = Sha256::new();
    prefix.update(WORK_CONTEXT);
    prefix.update(account.as_bytes());
    prefix.update(name);
    let mut counter: u64 = 0;
    loop {
        let work = counter.to_be_bytes();
        let proof = prefix.clone().chain_update(work).finalize();
        if leading_zero_bits(&proof) >= u32::from(bits) {
            return work;
        }
        counter = counter.wrapping_add(1);
    }
}

fn leading_zero_bits(bytes: &[u8]) -> u32 {
    let mut bits = 0;
    for &byte in bytes {
        if byte != 0 {
            return bits + byte.leading_zeros();
        }
        bits += 8;
    }
    bits
}

fn json_request(method: &'static str, path: String, body: &serde_json::Value) -> Result<Request> {
    Ok(Request {
        method,
        path,
        headers: vec![("content-type", "application/json".to_owned())],
        body: serde_json::to_vec(body).map_err(|e| CoreError::Encoding {
            what: "request",
            source: e.into(),
        })?,
    })
}

/// `Ok` for an expected status; otherwise the service's error code.
pub(crate) fn expect_success(status: u16, body: &[u8], expected: &[u16]) -> Result<()> {
    #[derive(Deserialize)]
    struct Refusal {
        error: String,
    }
    if expected.contains(&status) {
        return Ok(());
    }
    let code = serde_json::from_slice::<Refusal>(body)
        .map_or_else(|_| "unexpected_answer".to_owned(), |refusal| refusal.error);
    Err(CoreError::Service { status, code })
}

pub(crate) fn parse<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T> {
    serde_json::from_slice(body)
        .map_err(|_| CoreError::malformed("service answer", "unexpected shape"))
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn unb64(text: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| CoreError::malformed("service answer", "invalid base64url"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use openmls::prelude::SignatureScheme;
    use openmls_rust_crypto::RustCrypto;
    use openmls_traits::crypto::OpenMlsCrypto;
    use serde::Serialize;

    use super::*;
    use crate::envelope::MailboxId;
    use crate::identity::DeviceId;
    use crate::store::MasterKey;

    #[test]
    fn usernames_follow_the_directory_rules() {
        for (input, expected) in username_cases() {
            assert_eq!(normalize_username(&input).ok(), expected, "{input:?}");
        }
    }

    #[test]
    fn proofs_of_work_have_the_requested_difficulty() {
        let account = AccountKey::generate().expect("account");
        let name = name_field("alice");
        let work = solve_work(&account.id(), &name, 12);
        let proof = Sha256::new()
            .chain_update(WORK_CONTEXT)
            .chain_update(account.id().as_bytes())
            .chain_update(&name)
            .chain_update(work)
            .finalize();
        assert!(leading_zero_bits(&proof) >= 12);
        assert_eq!(leading_zero_bits(&[0x00, 0x01]), 15);
        assert_eq!(leading_zero_bits(&[0x80]), 0);
        assert_eq!(leading_zero_bits(&[0, 0]), 16);
    }

    #[test]
    fn delivery_frames_need_one_to_255_recipients_and_a_bounded_payload() {
        let outgoing = |recipients: usize, payload: usize| Outgoing {
            id: EnvelopeId([1; 16]),
            recipients: vec![MailboxId([2; 32]); recipients],
            payload: vec![0; payload],
            commit: None,
        };
        assert!(delivery(&outgoing(0, 1)).is_err());
        assert!(delivery(&outgoing(256, 1)).is_err());
        assert!(delivery(&outgoing(255, MAX_PAYLOAD)).is_ok());
        assert!(matches!(
            delivery(&outgoing(1, MAX_PAYLOAD + 1)),
            Err(CoreError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_sequencer_answers_accept_or_refuse_and_nothing_else() {
        assert_eq!(
            commit_answer(202, br#"{"epoch":8,"delivered":1,"welcomed":1}"#).expect("ok"),
            CommitAnswer::Accepted { epoch: 8 }
        );
        assert_eq!(
            commit_answer(409, br#"{"error":"epoch_mismatch","epoch":9}"#).expect("ok"),
            CommitAnswer::Refused { epoch: 9 }
        );
        assert!(matches!(
            commit_answer(409, br#"{"error":"something_else"}"#),
            Err(CoreError::Service { status: 409, .. })
        ));
        assert!(commit_answer(503, b"").expect_err("busy").is_transient());
    }

    #[test]
    fn refusals_carry_the_service_code() {
        assert!(matches!(
            claimed(409, br#"{"error":"username_taken"}"#),
            Err(CoreError::Service { status: 409, code }) if code == "username_taken"
        ));
        let busy = delivered(503, b"<html>").expect_err("refused");
        assert!(busy.is_transient());
        assert!(matches!(busy, CoreError::Service { code, .. } if code == "unexpected_answer"));
        assert_eq!(delivered(202, br#"{"delivered":2}"#).expect("ok"), 2);
        assert!(delivered(202, b"{}").is_err());
    }

    #[test]
    fn a_directory_answer_must_only_list_the_accounts_own_devices() {
        let alice = AccountKey::generate().expect("account");
        let mallory = AccountKey::generate().expect("account");
        let device_of = |account: &AccountKey| {
            account
                .certify_device(DeviceId::random().expect("id"), &[7; 32])
                .expect("certificate")
        };
        let answer = |certificate: &DeviceCertificate| {
            serde_json::to_vec(&json!({
                "username": "alice",
                "account": b64(alice.id().as_bytes()),
                "devices": [{ "certificate": b64(&certificate.encode()), "key_package": b64(b"kp") }],
            }))
            .expect("json")
        };

        let found = directory_entry("Alice", 200, &answer(&device_of(&alice))).expect("entry");
        assert_eq!(found.username, "alice");
        assert_eq!(found.account, alice.id());
        assert_eq!(found.key_packages, vec![b"kp".to_vec()]);
        assert!(matches!(
            directory_entry("alice", 200, &answer(&device_of(&mallory))),
            Err(CoreError::Unauthenticated(_))
        ));
        assert!(matches!(
            directory_entry("bob", 200, &answer(&device_of(&alice))),
            Err(CoreError::Malformed { .. })
        ));
        assert!(matches!(
            directory_entry("alice", 404, br#"{"error":"unknown_account"}"#),
            Err(CoreError::Service { status: 404, .. })
        ));
    }

    #[test]
    fn mailbox_access_never_prints_the_secret() {
        let dir = tempfile::tempdir().expect("dir");
        let account = AccountKey::generate().expect("account");
        let device = Device::create(
            &dir.path().join("d.db"),
            &MasterKey::generate().expect("key"),
            &account,
        )
        .expect("device");
        let access = device.mailbox_access();
        let secret = access
            .authorization
            .strip_prefix("Bearer ")
            .expect("bearer");
        assert!(!format!("{access:?}").contains(secret));
    }

    // ---- Shared protocol vectors (protocol/vectors/v0.json) ------------------

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Vectors {
        about: String,
        usernames: Vec<(String, Option<String>)>,
        claim: ClaimVector,
        request: RequestVector,
        delivery: DeliveryVector,
        commit: CommitVector,
        envelope: EnvelopeVector,
        acknowledgement: AcknowledgementVector,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct ClaimVector {
        account_secret: String,
        username: String,
        invite: String,
        timestamp: u64,
        work_bits: u8,
        body: serde_json::Value,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct RequestVector {
        method: String,
        path: String,
        timestamp: u64,
        headers: BTreeMap<String, String>,
        body: String,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct DeliveryVector {
        id: String,
        recipients: Vec<String>,
        payload: String,
        frame: String,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct CommitVector {
        group: String,
        epoch: u64,
        commit_id: String,
        welcome_id: String,
        welcome_recipients: Vec<String>,
        path: String,
        frame: String,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct EnvelopeVector {
        id: String,
        mailbox: String,
        payload: String,
        encoded: String,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct AcknowledgementVector {
        ids: Vec<String>,
        frame: String,
    }

    const VECTORS: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol/vectors/v0.json"
    );
    const TIMESTAMP: u64 = 1_760_000_000_000;

    fn username_cases() -> Vec<(String, Option<String>)> {
        let ok = |input: &str, normalized: &str| (input.to_owned(), Some(normalized.to_owned()));
        let bad = |input: &str| (input.to_owned(), None);
        vec![
            ok("Alice_92", "alice_92"),
            ok("a_b_c", "a_b_c"),
            ok("abc", "abc"),
            ok(&"a".repeat(32), &"a".repeat(32)),
            bad("ab"),
            bad(&"a".repeat(33)),
            bad("_abc"),
            bad("9abc"),
            bad("abc_"),
            bad("a__b"),
            bad("ab.c"),
            bad("a\u{f1}b"),
            // KELVIN SIGN: some lowercase functions turn it into an ASCII k.
            bad("\u{212A}ate"),
        ]
    }

    fn fixed_delivery() -> Outgoing {
        Outgoing {
            id: EnvelopeId([0x11; 16]),
            recipients: vec![MailboxId([0x22; 32]), MailboxId([0x33; 32])],
            payload: b"opaque MLS message".to_vec(),
            commit: None,
        }
    }

    fn fixed_commit() -> Outgoing {
        Outgoing {
            id: EnvelopeId([0x88; 16]),
            recipients: vec![],
            payload: b"opaque MLS commit".to_vec(),
            commit: Some(crate::device::CommitTicket {
                group: vec![0x99; 16],
                epoch: 7,
                welcome: Some(Box::new(Outgoing {
                    id: EnvelopeId([0xAA; 16]),
                    recipients: vec![MailboxId([0xBB; 32])],
                    payload: b"opaque MLS welcome".to_vec(),
                    commit: None,
                })),
            }),
        }
    }

    fn fixed_envelope() -> Envelope {
        Envelope {
            id: EnvelopeId([0x44; 16]),
            to: MailboxId([0x55; 32]),
            payload: b"opaque MLS message".to_vec(),
        }
    }

    fn fixed_acknowledged() -> Vec<EnvelopeId> {
        vec![EnvelopeId([0x66; 16]), EnvelopeId([0x77; 16])]
    }

    fn body_json(request: &Request) -> serde_json::Value {
        serde_json::from_slice(&request.body).expect("json body")
    }

    /// Builds the vectors from fresh keys (only when asked to write them).
    fn generate() -> Vectors {
        let account = AccountKey::generate().expect("account");
        let invite = "abcde-fghij-klmno-pqrst";
        let claim_request = claim(&account, "Alice_92", invite, TIMESTAMP, 8).expect("claim");

        let dir = tempfile::tempdir().expect("dir");
        let mut device = Device::create(
            &dir.path().join("vectors.db"),
            &MasterKey::generate().expect("key"),
            &account,
        )
        .expect("device");
        let request = device
            .replenish_key_packages("alice_92", 1, TIMESTAMP)
            .expect("request");

        let outgoing = fixed_delivery();
        let envelope = fixed_envelope();
        let commit_request = delivery(&fixed_commit()).expect("commit");
        Vectors {
            about: "Zelamar protocol v0 test vectors. Generated by zelamar-core \
                    (ZELAMAR_WRITE_VECTORS=1 cargo test -p zelamar-core vectors), \
                    checked by the core and by the server. Keys are test-only."
                .to_owned(),
            usernames: username_cases(),
            claim: ClaimVector {
                account_secret: b64(&account.to_secret_bytes().expect("secret")),
                username: "Alice_92".to_owned(),
                invite: invite.to_owned(),
                timestamp: TIMESTAMP,
                work_bits: 8,
                body: body_json(&claim_request),
            },
            request: RequestVector {
                method: request.method.to_owned(),
                path: request.path.clone(),
                timestamp: TIMESTAMP,
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), value.clone()))
                    .collect(),
                body: b64(&request.body),
            },
            delivery: DeliveryVector {
                id: b64(&outgoing.id.0),
                recipients: outgoing.recipients.iter().map(|r| b64(&r.0)).collect(),
                payload: b64(&outgoing.payload),
                frame: b64(&delivery(&outgoing).expect("frame").body),
            },
            commit: CommitVector {
                group: b64(&[0x99; 16]),
                epoch: 7,
                commit_id: b64(&[0x88; 16]),
                welcome_id: b64(&[0xAA; 16]),
                welcome_recipients: vec![b64(&[0xBB; 32])],
                path: commit_request.path,
                frame: b64(&commit_request.body),
            },
            envelope: EnvelopeVector {
                id: b64(&envelope.id.0),
                mailbox: b64(&envelope.to.0),
                payload: b64(&envelope.payload),
                encoded: b64(&envelope.encode().expect("envelope")),
            },
            acknowledgement: AcknowledgementVector {
                ids: fixed_acknowledged().iter().map(|id| b64(&id.0)).collect(),
                frame: b64(&acknowledgement(&fixed_acknowledged())),
            },
        }
    }

    /// The checked-in vectors still describe exactly what this code does.
    #[test]
    fn vectors() {
        if std::env::var_os("ZELAMAR_WRITE_VECTORS").is_some() {
            let json = serde_json::to_string_pretty(&generate()).expect("json");
            std::fs::write(VECTORS, json + "\n").expect("write vectors");
        }
        let vectors: Vectors =
            serde_json::from_str(&std::fs::read_to_string(VECTORS).expect("read vectors"))
                .expect("parse vectors");

        assert_eq!(vectors.usernames, username_cases());

        // The claim is deterministic given the key: rebuild it byte for byte.
        let claim_vector = &vectors.claim;
        let account =
            AccountKey::from_secret_bytes(&unb64(&claim_vector.account_secret).expect("b64"))
                .expect("account");
        let rebuilt = claim(
            &account,
            &claim_vector.username,
            &claim_vector.invite,
            claim_vector.timestamp,
            claim_vector.work_bits,
        )
        .expect("claim");
        assert_eq!(body_json(&rebuilt), claim_vector.body);

        // The signed request verifies under the rules of protocol/directory.md.
        let request = &vectors.request;
        let header = |name: &str| request.headers.get(name).expect("header").as_str();
        assert_eq!(header("zelamar-timestamp"), request.timestamp.to_string());
        let certificate =
            DeviceCertificate::decode(&unb64(header("zelamar-certificate")).expect("b64"))
                .expect("certificate");
        certificate.verify().expect("certificate verifies");
        let payload = [
            REQUEST_CONTEXT,
            request.method.as_bytes(),
            &[0],
            request.path.as_bytes(),
            &[0],
            &request.timestamp.to_be_bytes(),
            &Sha256::digest(unb64(&request.body).expect("b64")),
        ]
        .concat();
        RustCrypto::default()
            .verify_signature(
                SignatureScheme::ED25519,
                &payload,
                certificate.signature_key(),
                &unb64(header("zelamar-signature")).expect("b64"),
            )
            .expect("request signature verifies");

        assert_eq!(
            vectors.delivery.frame,
            b64(&delivery(&fixed_delivery()).expect("frame").body)
        );
        let commit_request = delivery(&fixed_commit()).expect("commit");
        assert_eq!(vectors.commit.frame, b64(&commit_request.body));
        assert_eq!(vectors.commit.path, commit_request.path);
        assert_eq!(
            vectors.envelope.encoded,
            b64(&fixed_envelope().encode().expect("envelope"))
        );
        assert_eq!(
            vectors.acknowledgement.frame,
            b64(&acknowledgement(&fixed_acknowledged()))
        );
    }
}
