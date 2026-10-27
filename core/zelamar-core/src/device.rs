//! A device as a participant in MLS chats (ADR-0003, ADR-0010, ADR-0011).
//!
//! Every chat is an MLS group, a 1:1 chat included. A [`Device`] owns its MLS
//! signature key, a certificate from its account, the mailbox it receives
//! at, and the state and history of every chat it belongs to. It turns
//! plaintext [`AppMessage`]s into opaque MLS bytes addressed to the other
//! members' mailboxes, and back. It refuses any member whose device
//! certificate does not verify or who publishes no mailbox.
//!
//! A device lives in an encrypted store on disk. Every operation is a unit of
//! work: on success its changes (MLS state, history, outbox) are persisted in
//! one transaction; on failure the in-memory state is rolled back to the last
//! persisted state, so a device is never left half-way through a change.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use openmls::prelude::{tls_codec::Deserialize as _, tls_codec::Serialize as _, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::{OpenMlsProvider, random::OpenMlsRand, signatures::Signer};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::contacts::{self, Contact, Contacts};
use crate::content::{AppMessage, Body, MAX_RECEIPT_IDS, MessageId, ReceiptStatus};
use crate::envelope::{EnvelopeId, MailboxId};
use crate::error::{CoreError, Result};
use crate::history::{self, HistoryEntry, HistoryReader, SendStatus, StoredMessage};
use crate::identity::{AccountId, AccountKey, DeviceCertificate, DeviceId};
use crate::link::{AnswerBody, Applicant, Joining, Linked, Offer};
use crate::recovery::{self, KitBody, KitUpload, Recovered, RecoveryKey};
use crate::service::{self, CommitAnswer, DirectoryEntry};
use crate::store::{Batch, MasterKey, Store};

/// The one ciphersuite Zelamar speaks: hybrid post-quantum key exchange
/// (ML-KEM-768 + X25519), AES-256-GCM, SHA-384, Ed25519 signatures.
///
/// It comes from the IETF MLS post-quantum draft. Zelamar controls every
/// client, so if the draft changes, new chats move to the final codepoint.
pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519;

/// Messages are padded to a multiple of this many bytes so their length
/// leaks less about their content.
const PADDING: usize = 64;

/// Past epochs whose keys are kept, so a message sent just before a change
/// in the chat (and delivered just after) still opens.
const MAX_PAST_EPOCHS: usize = 4;

/// Leaf-node extension carrying a device's mailbox id (private-use range).
const MAILBOX_EXTENSION: u16 = 0xF5A1;

const RECORD_CERTIFICATE: &str = "device.certificate";
const RECORD_ACCOUNT_KEY: &str = "account.key";
const RECORD_MAILBOX: &str = "device.mailbox";
const RECORD_MAILBOX_SECRET: &str = "device.mailbox_secret";
const RECORD_CHATS: &str = "device.chats";
const RECORD_USERNAME: &str = "account.username";
const RECORD_CONTACTS: &str = "device.contacts";
const RECORD_INTRODUCED: &str = "device.introduced";
const RECORD_READ_MARKS: &str = "device.read_marks";
const RECORD_LABELS: &str = "device.labels";
const RECORD_CHAT_META: &str = "device.chat_meta";
const RECORD_RECOVERY: &str = "account.recovery";

/// Most accounts in one chat (ADR-0015).
pub const MAX_MEMBERS: usize = 50;

/// Unread counts stop at this many; the app shows "99+".
const UNREAD_CAP: u32 = 99;

/// Identifier of a chat: the MLS group id (16 random bytes).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChatId(#[serde(with = "serde_bytes")] Vec<u8>);

impl ChatId {
    /// The raw id bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn of(group_id: &GroupId) -> Self {
        Self(group_id.as_slice().to_vec())
    }
}

/// Who sent a message: an authenticated account and one of its devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    /// The account, as proven by its device certificate.
    pub account: AccountId,
    /// The device that sent the message.
    pub device: DeviceId,
}

/// One ciphertext on its way out: the same `payload` goes to every mailbox
/// in `recipients`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outgoing {
    /// Deduplication id, and the handle for [`Device::settle`].
    pub id: EnvelopeId,
    /// The other members' devices; never this one.
    pub recipients: Vec<MailboxId>,
    /// The MLS message.
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
    /// Set for a commit others must apply: it goes through the chat's
    /// sequencer (`protocol/delivery.md`) instead of straight to mailboxes.
    #[serde(default)]
    pub commit: Option<CommitTicket>,
    /// Wakes a device that is away at once, as a ringing call must
    /// (ADR-0019). The service learns only that this envelope is urgent.
    #[serde(default)]
    pub urgent: bool,
}

/// What the sequencer needs to know about a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitTicket {
    /// The chat's MLS group id.
    #[serde(with = "serde_bytes")]
    pub group: Vec<u8>,
    /// The epoch the commit was made in.
    pub epoch: u64,
    /// The welcome for added devices, sent once the commit wins.
    pub welcome: Option<Box<Outgoing>>,
}

/// What became of an outbox item once the service answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Delivered to the mailboxes.
    Delivered,
    /// A commit that won its epoch: applied here too.
    Applied,
    /// A commit another member beat: dropped. The winner arrives in the
    /// mailbox; whatever the commit was for has to be done again on top.
    Refused,
}

/// Result of an invitation: the commit for existing members and the welcome
/// for the newcomer. Both are already queued in the outbox.
#[derive(Debug, Clone)]
pub struct Invitation {
    /// Goes to everyone already in the chat (no one, in a new chat).
    pub commit: Outgoing,
    /// Goes to the invited device only.
    pub welcome: Outgoing,
}

/// What an incoming MLS message turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Received {
    /// A message from another member.
    Message {
        /// The chat it belongs to.
        chat: ChatId,
        /// Its authenticated author.
        author: Author,
        /// The content.
        message: AppMessage,
    },
    /// The chat's membership or keys changed; already applied.
    ChatUpdated {
        /// The chat that changed.
        chat: ChatId,
    },
    /// A change to the chat took this device out of it: it can read and
    /// write nothing more there.
    Left {
        /// The chat this device is no longer in.
        chat: ChatId,
    },
    /// This device's own message, echoed back by a transport. Nothing to do:
    /// the device applied it when sending.
    OwnEcho {
        /// The chat it belongs to.
        chat: ChatId,
    },
    /// Someone added this device to a chat. The app shows it as a chat
    /// request until the user accepts it.
    Joined {
        /// The new chat.
        chat: ChatId,
        /// Who added this device, authenticated like any member.
        by: Author,
    },
}

/// One chat as the chat list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSummary {
    /// The chat.
    pub chat: ChatId,
    /// Everyone else in it, one entry per account.
    pub members: Vec<Contact>,
    /// Its newest message, if any.
    pub last: Option<HistoryEntry>,
    /// Messages from others not yet read here, up to 99.
    pub unread: u32,
    /// Someone else started it and the user has not answered yet: show it
    /// as a request to accept or decline.
    pub request: bool,
    /// The chat's name, for a group someone named.
    pub name: Option<String>,
}

/// What a device keeps about a chat beyond MLS and history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ChatMeta {
    /// The newest name: when it was written, by which message, and the name.
    #[serde(default)]
    name: Option<(u64, MessageId, String)>,
    /// Accounts that left, whose devices may still be in the chat.
    #[serde(default)]
    departed: BTreeSet<AccountId>,
}

/// The recovery key this device made, and the digest of the kit the service
/// last took (ADR-0018).
#[derive(Clone, Serialize, Deserialize)]
struct RecoveryRecord {
    code: String,
    #[serde(default)]
    uploaded: Option<[u8; 32]>,
}

impl Drop for RecoveryRecord {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.code);
    }
}

/// Devices of another account that a chat lacks while none of that
/// account's devices left in it is still registered, as after a recovery
/// (ADR-0018).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignGap {
    /// The chat.
    pub chat: ChatId,
    /// The account.
    pub account: AccountId,
    /// Its registered devices, to add.
    pub devices: Vec<DeviceId>,
    /// Whether this device holds the lowest registered leaf among the other
    /// members, and so is the one to add them.
    pub first: bool,
}

/// One Zelamar device: its keys and the chats it takes part in.
pub struct Device {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    certificate: DeviceCertificate,
    /// Present on the device that created the account; secondary devices
    /// (linked later) do not hold the account's root key.
    account_key: Option<AccountKey>,
    /// Where this device receives mail; published in every chat it is in.
    mailbox: MailboxId,
    /// Proves to the delivery service that this device owns `mailbox`.
    mailbox_secret: Zeroizing<[u8; 32]>,
    chats: HashMap<ChatId, MlsGroup>,
    store: Store,
    /// The MLS state exactly as it is on disk.
    persisted: HashMap<Vec<u8>, Vec<u8>>,
    /// The chat ids exactly as they are on disk.
    persisted_chats: HashSet<ChatId>,
    /// History and outbox writes of the operation in progress.
    pending: Pending,
    /// The username this account holds, once claimed.
    username: Option<String>,
    /// Who is who, in memory and exactly as on disk.
    contacts: Contacts,
    persisted_contacts: Contacts,
    /// Chats this device has sent its profile to, in memory and as on disk.
    /// Only these get receipts: until the user writes in a chat, the people
    /// there learn nothing, not even that a message arrived.
    introduced: BTreeSet<ChatId>,
    persisted_introduced: BTreeSet<ChatId>,
    /// Per chat, the newest history position the user has seen, in memory
    /// and as on disk.
    read_marks: BTreeMap<ChatId, i64>,
    persisted_read_marks: BTreeMap<ChatId, i64>,
    /// What this account's devices call themselves, as far as this one
    /// knows (ADR-0014).
    labels: BTreeMap<DeviceId, String>,
    /// Names and departures per chat, in memory and as on disk.
    meta: BTreeMap<ChatId, ChatMeta>,
    persisted_meta: BTreeMap<ChatId, ChatMeta>,
    /// The recovery key, on the device holding the account key.
    recovery: Option<RecoveryRecord>,
}

/// Freshly built key packages, TLS-encoded.
pub(crate) struct KeyPackages {
    /// Each used once, then gone.
    pub(crate) one_time: Vec<Vec<u8>>,
    /// Reused once the one-time ones run out.
    pub(crate) last_resort: Option<Vec<u8>>,
}

/// Writes of the operation in progress, joined into its transaction.
#[derive(Default)]
struct Pending {
    messages: Vec<(ChatId, StoredMessage)>,
    outbox: Vec<(EnvelopeId, Vec<u8>)>,
    confirmed: Vec<EnvelopeId>,
    erased: Vec<ChatId>,
}

/// An outbox item: what to send, and the history record it advances.
#[derive(Serialize, Deserialize)]
struct Queued {
    outgoing: Outgoing,
    /// The message whose status becomes `Sent` once delivered.
    message: Option<(ChatId, MessageId)>,
}

impl Queued {
    fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).map_err(|e| CoreError::Encoding {
            what: "outbox item",
            source: e.into(),
        })?;
        Ok(out)
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        ciborium::from_reader(bytes)
            .map_err(|_| CoreError::malformed("outbox item", "invalid encoding"))
    }
}

impl Device {
    /// Creates a new device for `account` in a fresh store at `path`: a new
    /// MLS signature key, certified by the account key, persisted under
    /// `master`.
    ///
    /// # Errors
    ///
    /// [`CoreError::AlreadyInitialized`] if the store already holds a device,
    /// [`CoreError::WrongKey`] if `master` does not open an existing file,
    /// or a crypto, MLS or storage error.
    pub fn create(path: &Path, master: &MasterKey, account: &AccountKey) -> Result<Self> {
        let store = Self::fresh_store(path, master)?;
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|e| CoreError::crypto("generating the device signature key", e))?;
        let certificate = account.certify_device(DeviceId::random()?, signer.public())?;
        let account_key = AccountKey::from_secret_bytes(&account.to_secret_bytes()?)?;
        let mut device = Self::assemble(store, signer, certificate, Some(account_key))?;
        let records = device.base_records()?;
        device.persist_with(records)?;
        Ok(device)
    }

    /// Creates a device linked to an existing account (ADR-0014): the keys
    /// `joining` made, and what the account's device answered. It never
    /// holds the account key.
    ///
    /// # Errors
    ///
    /// As [`Device::create`].
    pub fn create_linked(
        path: &Path,
        master: &MasterKey,
        joining: Joining,
        linked: Linked,
    ) -> Result<Self> {
        let store = Self::fresh_store(path, master)?;
        let mut device = Self::assemble(store, joining.signer, linked.certificate, None)?;
        let own = device.account();
        device.contacts.extend(
            linked
                .contacts
                .into_iter()
                .filter(|contact| contact.account != own)
                .map(|contact| (contact.account, contact)),
        );
        device.introduced.extend(linked.introduced);
        device.username = Some(linked.username);
        device.labels.extend(linked.labels);
        let records = device.base_records()?;
        device.persist_with(records)?;
        Ok(device)
    }

    /// Creates a device for an account recovered from its kit (ADR-0018):
    /// a new device certified by the recovered account key, which it then
    /// holds, as the account's first device did.
    ///
    /// # Errors
    ///
    /// As [`Device::create`].
    pub fn create_recovered(path: &Path, master: &MasterKey, recovered: Recovered) -> Result<Self> {
        let store = Self::fresh_store(path, master)?;
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|e| CoreError::crypto("generating the device signature key", e))?;
        let certificate = recovered
            .account
            .certify_device(DeviceId::random()?, signer.public())?;
        let mut device = Self::assemble(store, signer, certificate, Some(recovered.account))?;
        let own = device.account();
        device.contacts.extend(
            recovered
                .contacts
                .into_iter()
                .filter(|contact| contact.account != own)
                .map(|contact| (contact.account, contact)),
        );
        device.introduced.extend(recovered.introduced);
        device.username = Some(recovered.username);
        device.labels.extend(recovered.labels);
        device.recovery = Some(RecoveryRecord {
            code: recovered.key.code().to_owned(),
            uploaded: None,
        });
        let records = device.base_records()?;
        device.persist_with(records)?;
        Ok(device)
    }

    fn fresh_store(path: &Path, master: &MasterKey) -> Result<Store> {
        let store = Store::open(path, master)?;
        if store.record(RECORD_CERTIFICATE)?.is_some() {
            return Err(CoreError::AlreadyInitialized);
        }
        Ok(store)
    }

    /// A new device around `signer` and its certificate, with a fresh
    /// mailbox. Nothing is persisted yet.
    fn assemble(
        store: Store,
        signer: SignatureKeyPair,
        certificate: DeviceCertificate,
        account_key: Option<AccountKey>,
    ) -> Result<Self> {
        let provider = OpenMlsRustCrypto::default();
        signer
            .store(provider.storage())
            .map_err(|e| CoreError::mls("storing the device signature key", e))?;
        let mailbox = MailboxId::random()?;
        let mailbox_secret = Zeroizing::new(
            provider
                .rand()
                .random_array::<32>()
                .map_err(|e| CoreError::crypto("drawing a mailbox secret", e))?,
        );
        Ok(Self {
            credential: credential_for(&certificate, &signer),
            provider,
            signer,
            certificate,
            account_key,
            mailbox,
            mailbox_secret,
            chats: HashMap::new(),
            store,
            persisted: HashMap::new(),
            persisted_chats: HashSet::new(),
            pending: Pending::default(),
            username: None,
            contacts: Contacts::new(),
            persisted_contacts: Contacts::new(),
            introduced: BTreeSet::new(),
            persisted_introduced: BTreeSet::new(),
            read_marks: BTreeMap::new(),
            persisted_read_marks: BTreeMap::new(),
            labels: BTreeMap::new(),
            meta: BTreeMap::new(),
            persisted_meta: BTreeMap::new(),
            recovery: None,
        })
    }

    /// What a new device writes once: who it is and where it receives.
    fn base_records(&self) -> Result<Vec<(&'static str, Vec<u8>)>> {
        let mut records = vec![
            (RECORD_CERTIFICATE, self.certificate.encode()),
            (RECORD_MAILBOX, self.mailbox.0.to_vec()),
            (RECORD_MAILBOX_SECRET, self.mailbox_secret.to_vec()),
        ];
        if let Some(account) = &self.account_key {
            records.push((RECORD_ACCOUNT_KEY, account.to_secret_bytes()?.to_vec()));
        }
        if let Some(username) = &self.username {
            records.push((RECORD_USERNAME, username.as_bytes().to_vec()));
        }
        if !self.labels.is_empty() {
            records.push((RECORD_LABELS, encode_labels(&self.labels)?));
        }
        if let Some(recovery) = &self.recovery {
            records.push((RECORD_RECOVERY, encode_recovery(recovery)?));
        }
        Ok(records)
    }

    /// Opens the device stored at `path`.
    ///
    /// # Errors
    ///
    /// [`CoreError::WrongKey`] if `master` does not open the store,
    /// [`CoreError::NotInitialized`] if it holds no device, or a storage or
    /// integrity error if the file was damaged or tampered with.
    pub fn open(path: &Path, master: &MasterKey) -> Result<Self> {
        let store = Store::open(path, master)?;
        let certificate = DeviceCertificate::decode(
            &store
                .record(RECORD_CERTIFICATE)?
                .ok_or(CoreError::NotInitialized)?,
        )?;
        let account_key = store
            .record(RECORD_ACCOUNT_KEY)?
            .map(|bytes| AccountKey::from_secret_bytes(&bytes))
            .transpose()?;
        let mailbox = MailboxId(
            store
                .record(RECORD_MAILBOX)?
                .ok_or(CoreError::malformed("store", "device mailbox is missing"))?
                .as_slice()
                .try_into()
                .map_err(|_| CoreError::malformed("store", "device mailbox is malformed"))?,
        );
        let mailbox_secret = Zeroizing::new(
            <[u8; 32]>::try_from(
                store
                    .record(RECORD_MAILBOX_SECRET)?
                    .ok_or(CoreError::malformed("store", "mailbox secret is missing"))?
                    .as_slice(),
            )
            .map_err(|_| CoreError::malformed("store", "mailbox secret is malformed"))?,
        );
        let chat_ids = decode_chat_list(&store.record(RECORD_CHATS)?.unwrap_or_default())?;
        let username = store
            .record(RECORD_USERNAME)?
            .map(|bytes| {
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| CoreError::malformed("store", "username is not UTF-8"))
            })
            .transpose()?;
        let contacts = decoded_or_default(&store, RECORD_CONTACTS, contacts::decode)?;
        let introduced: BTreeSet<ChatId> =
            decode_chat_list(&store.record(RECORD_INTRODUCED)?.unwrap_or_default())?
                .into_iter()
                .collect();
        let read_marks = decoded_or_default(&store, RECORD_READ_MARKS, decode_read_marks)?;
        let labels = decoded_or_default(&store, RECORD_LABELS, decode_labels)?;
        let meta = decoded_or_default(&store, RECORD_CHAT_META, decode_meta)?;
        let recovery = store
            .record(RECORD_RECOVERY)?
            .map(|bytes| decode_recovery(&bytes))
            .transpose()?;

        let provider = OpenMlsRustCrypto::default();
        let persisted = store.load_mls_state()?;
        provider
            .storage()
            .values
            .write()
            .map_err(|_| poisoned())?
            .clone_from(&persisted);

        let signer = SignatureKeyPair::read(
            provider.storage(),
            certificate.signature_key(),
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(CoreError::malformed(
            "store",
            "device signature key is missing",
        ))?;
        let chats = load_chats(&provider, &chat_ids)?;

        Ok(Self {
            credential: credential_for(&certificate, &signer),
            provider,
            signer,
            certificate,
            account_key,
            mailbox,
            mailbox_secret,
            chats,
            store,
            persisted,
            persisted_chats: chat_ids,
            pending: Pending::default(),
            username,
            persisted_contacts: contacts.clone(),
            contacts,
            persisted_introduced: introduced.clone(),
            introduced,
            persisted_read_marks: read_marks.clone(),
            read_marks,
            labels,
            persisted_meta: meta.clone(),
            meta,
            recovery,
        })
    }

    /// The account this device belongs to.
    #[must_use]
    pub const fn account(&self) -> AccountId {
        self.certificate.account()
    }

    /// This device's id.
    #[must_use]
    pub const fn id(&self) -> DeviceId {
        self.certificate.device()
    }

    /// Whether this device holds its account's root key (the device that
    /// created the account does; linked devices do not).
    #[must_use]
    pub const fn holds_account_key(&self) -> bool {
        self.account_key.is_some()
    }

    /// The username this account holds, once claimed through this device.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// What `device`, one of this account's, calls itself, if this device
    /// knows.
    #[must_use]
    pub fn device_label(&self, device: DeviceId) -> Option<&str> {
        self.labels.get(&device).map(String::as_str)
    }

    /// Remembers what `device`, one of this account's, calls itself; each
    /// device tells its own, and linking passes them on.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for an overlong label, or a storage error.
    pub fn remember_label(&mut self, device: DeviceId, label: &str) -> Result<()> {
        if label.chars().count() > crate::link::MAX_LABEL {
            return Err(CoreError::malformed("device label", "too long"));
        }
        if self.labels.get(&device).map(String::as_str) == Some(label) {
            return Ok(());
        }
        let mut labels = self.labels.clone();
        labels.insert(device, label.to_owned());
        self.store.apply(&Batch {
            records: vec![(RECORD_LABELS, encode_labels(&labels)?)],
            ..Batch::default()
        })?;
        self.labels = labels;
        Ok(())
    }

    /// The recovery key, as shown (ADR-0018); only the device holding the
    /// account key has one.
    #[must_use]
    pub fn recovery_key(&self) -> Option<String> {
        self.recovery
            .as_ref()
            .and_then(|record| RecoveryKey::parse(&record.code).ok())
            .map(|key| key.display())
    }

    /// Makes a new recovery key, replacing any earlier one, and returns it
    /// as shown. The service takes the kit at the next
    /// [`Device::recovery_upload`]; the old key stops working then.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] on a device without the account key,
    /// or a crypto or storage error.
    pub fn new_recovery_key(&mut self) -> Result<String> {
        if self.account_key.is_none() {
            return Err(CoreError::Unauthenticated(
                "only the device holding the account key makes recovery keys",
            ));
        }
        let key = RecoveryKey::generate()?;
        let record = RecoveryRecord {
            code: key.code().to_owned(),
            uploaded: None,
        };
        self.store.apply(&Batch {
            records: vec![(RECORD_RECOVERY, encode_recovery(&record)?)],
            ..Batch::default()
        })?;
        self.recovery = Some(record);
        Ok(key.display())
    }

    /// The kit to leave at the service, when it no longer holds what this
    /// device would put in it: the account key, the name, the contacts, the
    /// chats this account has spoken in and the device labels (ADR-0018).
    ///
    /// # Errors
    ///
    /// [`CoreError::TooLarge`], or a crypto or encoding error.
    pub fn recovery_upload(&self, now_ms: u64) -> Result<Option<KitUpload>> {
        let (Some(record), Some(account), Some(username)) =
            (&self.recovery, &self.account_key, &self.username)
        else {
            return Ok(None);
        };
        let kit = KitBody {
            account_key: account.to_secret_bytes()?.to_vec(),
            username: username.clone(),
            contacts: self.contacts(),
            introduced: self.introduced.iter().cloned().collect(),
            labels: self
                .labels
                .iter()
                .map(|(device, label)| (*device, label.clone()))
                .collect(),
        };
        let encoded = kit.encode()?;
        if record.uploaded == Some(encoded.1) {
            return Ok(None);
        }
        let key = RecoveryKey::parse(&record.code)?;
        recovery::upload(&key, account, username, encoded, now_ms).map(Some)
    }

    /// Reads the answer to a [`Device::recovery_upload`]: the service holds
    /// that kit now, unless the key changed in the meantime.
    ///
    /// # Errors
    ///
    /// [`CoreError::Service`], or a storage error.
    pub fn recovery_uploaded(
        &mut self,
        upload: &KitUpload,
        status: u16,
        body: &[u8],
    ) -> Result<()> {
        service::expect_success(status, body, &[200])?;
        let Some(record) = &self.recovery else {
            return Ok(());
        };
        if record.code != *upload.code {
            return Ok(());
        }
        let record = RecoveryRecord {
            code: record.code.clone(),
            uploaded: Some(upload.digest),
        };
        self.store.apply(&Batch {
            records: vec![(RECORD_RECOVERY, encode_recovery(&record)?)],
            ..Batch::default()
        })?;
        self.recovery = Some(record);
        Ok(())
    }

    /// Remembers the username the service granted.
    pub(crate) fn set_username(&mut self, username: &str) -> Result<()> {
        self.store.apply(&Batch {
            records: vec![(RECORD_USERNAME, username.as_bytes().to_vec())],
            ..Batch::default()
        })?;
        self.username = Some(username.to_owned());
        Ok(())
    }

    /// The account's root key, on the device that created the account.
    pub(crate) const fn account_key(&self) -> Option<&AccountKey> {
        self.account_key.as_ref()
    }

    /// Everyone this device knows.
    #[must_use]
    pub fn contacts(&self) -> Vec<Contact> {
        self.contacts.values().cloned().collect()
    }

    /// Contacts whose username only they themselves claim. Look each one up
    /// ([`crate::service::key_lookup`]) and pass the answer to
    /// [`Device::confirm_contact`].
    #[must_use]
    pub fn unconfirmed_contacts(&self) -> Vec<Contact> {
        self.contacts
            .values()
            .filter(|contact| !contact.verified && contact.username.is_some())
            .cloned()
            .collect()
    }

    /// Confirms a contact's claimed username with a directory answer.
    /// Returns whether the directory agreed with the claim.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn confirm_contact(&mut self, entry: &DirectoryEntry) -> Result<bool> {
        let name = entry.username.as_str();
        let agrees = self
            .contacts
            .get(&entry.account)
            .is_some_and(|contact| contact.username.as_deref() == Some(name));
        // Anyone else claiming the name is impersonating its holder.
        let impostors = self.claimants(name, Some(entry.account));
        if agrees || !impostors.is_empty() {
            self.transact(|device| {
                for account in &impostors {
                    contacts::refuted(&mut device.contacts, *account, name);
                }
                if agrees {
                    contacts::confirmed(&mut device.contacts, entry.account, name);
                }
                Ok(())
            })?;
        }
        Ok(agrees)
    }

    /// Drops every unconfirmed claim of `username`, which the directory says
    /// no one holds.
    ///
    /// # Errors
    ///
    /// A storage error.
    pub fn refute_name(&mut self, username: &str) -> Result<()> {
        let claimants = self.claimants(username, None);
        if claimants.is_empty() {
            return Ok(());
        }
        self.transact(|device| {
            for account in &claimants {
                contacts::refuted(&mut device.contacts, *account, username);
            }
            Ok(())
        })
    }

    /// Accounts other than `except` claiming `username` without the
    /// directory's word.
    fn claimants(&self, username: &str, except: Option<AccountId>) -> Vec<AccountId> {
        self.contacts
            .values()
            .filter(|contact| {
                !contact.verified
                    && contact.username.as_deref() == Some(username)
                    && Some(contact.account) != except
            })
            .map(|contact| contact.account)
            .collect()
    }

    /// Every chat for the chat list, most recently active first.
    ///
    /// # Errors
    ///
    /// A storage or integrity error.
    pub fn chat_list(&self) -> Result<Vec<ChatSummary>> {
        let own = self.account();
        let mut list = self
            .chats
            .iter()
            .map(|(chat, group)| {
                let accounts = group
                    .members()
                    .map(|member| certificate_of(&member.credential).map(|c| c.account()))
                    .collect::<Result<BTreeSet<AccountId>>>()?;
                let members = accounts
                    .into_iter()
                    .filter(|account| *account != own)
                    .map(|account| {
                        self.contacts
                            .get(&account)
                            .cloned()
                            .unwrap_or_else(|| Contact::unknown(account))
                    })
                    .collect();
                Ok(ChatSummary {
                    chat: chat.clone(),
                    members,
                    last: self.history(chat, None, 1)?.into_iter().next(),
                    unread: self.unread(chat)?,
                    request: !self.introduced.contains(chat),
                    name: self.chat_name(chat).map(str::to_owned),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let activity = |summary: &ChatSummary| {
            summary
                .last
                .as_ref()
                .map(|entry| entry.message.received_at_ms)
        };
        list.sort_by(|a, b| {
            activity(b)
                .cmp(&activity(a))
                .then_with(|| a.chat.cmp(&b.chat))
        });
        Ok(list)
    }

    /// The mailbox this device receives at.
    #[must_use]
    pub const fn mailbox(&self) -> MailboxId {
        self.mailbox
    }

    /// Every other account in the chats this account has spoken in: the
    /// names whose devices healing looks up (ADR-0018).
    ///
    /// # Errors
    ///
    /// A malformed member.
    pub fn chat_accounts(&self) -> Result<BTreeSet<AccountId>> {
        let own = self.account();
        let mut accounts = BTreeSet::new();
        for (chat, group) in &self.chats {
            if !self.introduced.contains(chat) {
                continue;
            }
            for member in group.members() {
                let account = certificate_of(&member.credential)?.account();
                if account != own {
                    accounts.insert(account);
                }
            }
        }
        Ok(accounts)
    }

    /// Whether the user has taken part in `chat` here: started it, wrote in
    /// it or accepted it. Until then it is a request, and nothing from it
    /// rings (ADR-0019).
    #[must_use]
    pub fn accepted(&self, chat: &ChatId) -> bool {
        self.introduced.contains(chat)
    }

    /// Every chat this device takes part in, in a stable order.
    #[must_use]
    pub fn chats(&self) -> Vec<ChatId> {
        let mut chats: Vec<ChatId> = self.chats.keys().cloned().collect();
        chats.sort();
        chats
    }

    /// A page of `chat`'s history, newest first: up to `limit` messages
    /// older than `before` (the smallest [`HistoryEntry::seq`] of the
    /// previous page), or the newest ones when `before` is `None`.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or a storage or integrity error.
    pub fn history(
        &self,
        chat: &ChatId,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>> {
        if !self.chats.contains_key(chat) {
            return Err(CoreError::UnknownChat);
        }
        self.store
            .messages(chat.as_bytes(), before, limit)?
            .into_iter()
            .map(|(seq, bytes)| {
                Ok(HistoryEntry {
                    seq,
                    message: StoredMessage::decode(&bytes)?,
                })
            })
            .collect()
    }

    /// One message of `chat`'s history, if it is there.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or a storage or integrity error.
    pub fn message(&self, chat: &ChatId, id: MessageId) -> Result<Option<StoredMessage>> {
        if !self.chats.contains_key(chat) {
            return Err(CoreError::UnknownChat);
        }
        self.store
            .message(chat.as_bytes(), &id.0)?
            .map(|bytes| StoredMessage::decode(&bytes))
            .transpose()
    }

    /// Everything waiting to be sent, oldest first. A transport delivers each
    /// item, then calls [`Device::confirm_sent`]; until then the item stays
    /// queued, across restarts.
    ///
    /// # Errors
    ///
    /// A storage or integrity error.
    pub fn outbox(&self) -> Result<Vec<Outgoing>> {
        self.store
            .outbox()?
            .iter()
            .map(|bytes| Queued::decode(bytes).map(|item| item.outgoing))
            .collect()
    }

    /// Removes a delivered item from the outbox and marks its message as
    /// sent. Confirming an unknown or already confirmed id does nothing.
    ///
    /// # Errors
    ///
    /// A storage or integrity error.
    pub fn confirm_sent(&mut self, id: EnvelopeId) -> Result<()> {
        self.transact(|device| device.confirm(id))
    }

    /// Reads the service's answer to the outbox item `id` and acts on it.
    /// A delivery is confirmed. A commit the sequencer accepted is applied
    /// here; one it refused is dropped (the winning commit arrives in the
    /// mailbox). An answer for an item already settled changes nothing.
    ///
    /// # Errors
    ///
    /// [`CoreError::Service`] for any other answer (retry it later if
    /// [`CoreError::is_transient`]), or an MLS or storage error.
    pub fn settle(&mut self, id: EnvelopeId, status: u16, body: &[u8]) -> Result<Settled> {
        let Some(bytes) = self.store.outbox_item(&id.0)? else {
            return Ok(Settled::Delivered);
        };
        let item = Queued::decode(&bytes)?;
        let Some(ticket) = item.outgoing.commit else {
            service::delivered(status, body)?;
            self.confirm_sent(id)?;
            return Ok(Settled::Delivered);
        };
        let answer = service::commit_answer(status, body)?;
        let chat = ChatId(ticket.group);
        self.transact(|device| {
            device.pending.confirmed.push(id);
            // Act only on the commit this answer is about: one made in this
            // very epoch and still pending here.
            if let Some(group) = device.chats.get_mut(&chat)
                && group.pending_commit().is_some()
                && group.epoch().as_u64() == ticket.epoch
            {
                match answer {
                    CommitAnswer::Accepted { .. } => {
                        group
                            .merge_pending_commit(&device.provider)
                            .map_err(|e| CoreError::mls("applying an accepted commit", e))?;
                        if ticket.welcome.is_some() {
                            device.announce_name(&chat)?;
                        }
                    }
                    CommitAnswer::Refused { .. } => {
                        group
                            .clear_pending_commit(device.provider.storage())
                            .map_err(|e| CoreError::mls("dropping a refused commit", e))?;
                    }
                }
            }
            Ok(match answer {
                CommitAnswer::Accepted { .. } => Settled::Applied,
                CommitAnswer::Refused { .. } => Settled::Refused,
            })
        })
    }

    /// Removes a delivered item from the outbox and marks its message sent.
    fn confirm(&mut self, id: EnvelopeId) -> Result<()> {
        let Some(bytes) = self.store.outbox_item(&id.0)? else {
            return Ok(());
        };
        let item = Queued::decode(&bytes)?;
        self.pending.confirmed.push(id);
        if let Some((chat, message)) = item.message {
            let reader = ChatHistory {
                store: &self.store,
                chat: &chat,
                pending: &self.pending.messages,
            };
            if let Some(mut record) = reader.load(message)? {
                record.status = SendStatus::Sent;
                self.pending.messages.push((chat, record));
            }
        }
        Ok(())
    }

    /// Publishes a one-time `KeyPackage`: lets others add this device to a chat
    /// while it is offline. The private part stays in this device's storage.
    ///
    /// # Errors
    ///
    /// [`CoreError::Mls`] if the `KeyPackage` cannot be built or encoded, or a
    /// storage error.
    pub fn key_package(&mut self) -> Result<Vec<u8>> {
        self.transact(|device| device.build_key_package(false))
    }

    /// `count` one-time key packages and, when `last_resort` is set, one
    /// more that stays usable after the one-time ones run out. All are built
    /// in one unit of work.
    pub(crate) fn key_packages(&mut self, count: usize, last_resort: bool) -> Result<KeyPackages> {
        self.transact(|device| {
            Ok(KeyPackages {
                one_time: (0..count)
                    .map(|_| device.build_key_package(false))
                    .collect::<Result<Vec<_>>>()?,
                last_resort: last_resort
                    .then(|| device.build_key_package(true))
                    .transpose()?,
            })
        })
    }

    fn build_key_package(&self, last_resort: bool) -> Result<Vec<u8>> {
        let mut builder = KeyPackage::builder()
            .leaf_node_capabilities(capabilities())
            .leaf_node_extensions(mailbox_extension(self.mailbox)?);
        if last_resort {
            builder = builder.mark_as_last_resort();
        }
        let bundle = builder
            .build(
                CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential.clone(),
            )
            .map_err(|e| CoreError::mls("building a key package", e))?;
        bundle
            .key_package()
            .tls_serialize_detached()
            .map_err(|e| CoreError::mls("encoding a key package", e))
    }

    /// Signs a request to the delivery service with this device's key. Every
    /// signed payload starts with its own context string
    /// (`protocol/directory.md`), so it is never valid as anything else.
    pub(crate) fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        self.signer
            .sign(payload)
            .map_err(|e| CoreError::crypto("signing a request", format!("{e:?}")))
    }

    /// This device's certificate.
    pub(crate) const fn certificate(&self) -> &DeviceCertificate {
        &self.certificate
    }

    /// The secret that opens this device's mailbox at the delivery service.
    pub(crate) fn mailbox_secret(&self) -> &[u8; 32] {
        &self.mailbox_secret
    }

    /// Starts a new chat with this device as its only member.
    ///
    /// # Errors
    ///
    /// A crypto, MLS or storage error if the chat cannot be created.
    pub fn create_chat(&mut self) -> Result<ChatId> {
        self.transact(Self::new_chat)
    }

    /// Starts a chat with every device in `entry`, as the directory handed
    /// it out, and with this account's other devices in `siblings`, in one
    /// commit, and introduces this device there.
    ///
    /// Each device must be certified by the entry's account itself, so the
    /// directory cannot slip in someone else's device. Whether that account is
    /// really the person the user means is what safety codes confirm. This
    /// device itself is skipped in `siblings`, so a lookup of one's own name
    /// does as they are.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a device of another account,
    /// [`CoreError::Malformed`] for no or broken key packages,
    /// [`CoreError::Mls`] or a storage error.
    pub fn start_chat(
        &mut self,
        entry: &DirectoryEntry,
        siblings: &[DirectoryEntry],
    ) -> Result<(ChatId, Invitation)> {
        self.open_chat(std::slice::from_ref(entry), siblings, None)
    }

    /// Starts a group called `name` with every device of every account in
    /// `members`, and this account's other devices, in one commit
    /// (ADR-0015). Checks as [`Device::start_chat`].
    ///
    /// # Errors
    ///
    /// As [`Device::start_chat`]; [`CoreError::Malformed`] for a bad name,
    /// no members, or more than [`MAX_MEMBERS`].
    pub fn create_group(
        &mut self,
        name: &str,
        members: &[DirectoryEntry],
        siblings: &[DirectoryEntry],
    ) -> Result<(ChatId, Invitation)> {
        self.open_chat(members, siblings, Some(name))
    }

    fn open_chat(
        &mut self,
        members: &[DirectoryEntry],
        siblings: &[DirectoryEntry],
        name: Option<&str>,
    ) -> Result<(ChatId, Invitation)> {
        if members.is_empty() || members.len() + 1 > MAX_MEMBERS {
            return Err(CoreError::malformed(
                "chat",
                "no one, or too many, to talk to",
            ));
        }
        if members.iter().any(|entry| entry.key_packages.is_empty()) {
            return Err(CoreError::malformed("contact", "has no reachable device"));
        }
        if siblings
            .iter()
            .any(|sibling| sibling.account != self.account())
        {
            return Err(CoreError::Unauthenticated(
                "a sibling entry is about another account",
            ));
        }
        // The name goes out right after the chat starts; checked first.
        let named = match name {
            Some(name) => {
                let message = AppMessage::new(
                    MessageId::random()?,
                    now_ms(),
                    Body::Name {
                        name: name.to_owned(),
                    },
                );
                let bytes = message.encode()?;
                Some((message, bytes))
            }
            None => None,
        };
        self.transact(|device| {
            let (packages, newcomers) = device.checked_packages(members.iter().chain(siblings))?;
            let chat = device.new_chat()?;
            let invitation = device.add(&chat, &packages, newcomers)?;
            for entry in members {
                contacts::confirmed(&mut device.contacts, entry.account, &entry.username);
            }
            device.introduce(&chat)?;
            if let Some((message, bytes)) = &named {
                device.send_now(&chat, message, bytes)?;
            }
            Ok((chat, invitation))
        })
    }

    /// Adds every device of every account in `members` to `chat`, in one
    /// commit through the sequencer (ADR-0015).
    ///
    /// # Errors
    ///
    /// As [`Device::start_chat`]; [`CoreError::Malformed`] when the chat
    /// would hold more than [`MAX_MEMBERS`] accounts.
    pub fn add_members(&mut self, chat: &ChatId, members: &[DirectoryEntry]) -> Result<Invitation> {
        let group = self.chats.get(chat).ok_or(CoreError::UnknownChat)?;
        let mut accounts = group
            .members()
            .map(|member| certificate_of(&member.credential).map(|c| c.account()))
            .collect::<Result<BTreeSet<AccountId>>>()?;
        accounts.extend(members.iter().map(|entry| entry.account));
        if members.is_empty() || accounts.len() > MAX_MEMBERS {
            return Err(CoreError::malformed("chat", "no one, or too many, to add"));
        }
        self.transact(|device| {
            let (packages, newcomers) = device.checked_packages(members.iter())?;
            let invitation = device.add(chat, &packages, newcomers)?;
            let meta = device.meta.entry(chat.clone()).or_default();
            for entry in members {
                // Someone who left and comes back is a member again.
                meta.departed.remove(&entry.account);
                contacts::confirmed(&mut device.contacts, entry.account, &entry.username);
            }
            Ok(invitation)
        })
    }

    /// Every key package in `entries`, validated and certified by its
    /// entry's account, with the newcomers' mailboxes. This device itself
    /// is skipped.
    fn checked_packages<'a>(
        &self,
        entries: impl Iterator<Item = &'a DirectoryEntry>,
    ) -> Result<(Vec<KeyPackage>, Vec<MailboxId>)> {
        let me = self.id();
        let mut packages = Vec::new();
        let mut newcomers = Vec::new();
        for entry in entries {
            for bytes in &entry.key_packages {
                let key_package = self.validate_key_package(bytes)?;
                let leaf = key_package.leaf_node();
                let certificate = certificate_of(leaf.credential())?;
                if certificate.account() != entry.account {
                    return Err(CoreError::Unauthenticated(
                        "a device does not belong to the contact",
                    ));
                }
                if certificate.device() == me {
                    continue;
                }
                newcomers.push(verify_leaf(leaf)?);
                packages.push(key_package);
            }
        }
        Ok((packages, newcomers))
    }

    /// The name someone gave `chat`, if any (ADR-0015).
    #[must_use]
    pub fn chat_name(&self, chat: &ChatId) -> Option<&str> {
        self.meta
            .get(chat)
            .and_then(|meta| meta.name.as_ref())
            .map(|(_, _, name)| name.as_str())
    }

    /// Renames `chat` for everyone in it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for a bad name, [`CoreError::UnknownChat`],
    /// or an MLS or storage error.
    pub fn rename_chat(&mut self, chat: &ChatId, name: &str) -> Result<()> {
        let message = AppMessage::new(
            MessageId::random()?,
            now_ms(),
            Body::Name {
                name: name.to_owned(),
            },
        );
        self.send(chat, &message).map(|_| ())
    }

    /// Leaves `chat`: the others are told, so they take this account's
    /// devices out (ADR-0015), and this device erases it. A chat request is
    /// left without a word, like a declined one.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or a crypto, MLS or storage error.
    pub fn leave_chat(&mut self, chat: &ChatId) -> Result<()> {
        if !self.chats.contains_key(chat) {
            return Err(CoreError::UnknownChat);
        }
        self.transact(|device| {
            if device.introduced.contains(chat) {
                let leave = AppMessage::new(MessageId::random()?, now_ms(), Body::Leave);
                device.send_now(chat, &leave, &leave.encode()?)?;
            }
            device.erase(chat)
        })
    }

    /// Chats where accounts that left still have devices, and this device
    /// is the one to take them out: the remaining member with the lowest
    /// leaf, so the others do not race it (ADR-0015).
    ///
    /// # Errors
    ///
    /// A malformed member.
    pub fn departures(&self) -> Result<Vec<ChatId>> {
        let mut due = Vec::new();
        for (chat, meta) in &self.meta {
            if meta.departed.is_empty() {
                continue;
            }
            let Some(group) = self.chats.get(chat) else {
                continue;
            };
            if group.pending_commit().is_some() {
                continue;
            }
            let mut gone = false;
            let mut first_staying = None;
            for member in group.members() {
                let account = certificate_of(&member.credential)?.account();
                if meta.departed.contains(&account) {
                    gone = true;
                } else if first_staying.is_none() {
                    first_staying = Some(member.index);
                }
            }
            if gone && first_staying == Some(group.own_leaf_index()) {
                due.push(chat.clone());
            }
        }
        Ok(due)
    }

    /// Takes the devices of everyone who left `chat` out of it, in one
    /// commit through the sequencer.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], [`CoreError::Malformed`] when no one
    /// left, or an MLS or storage error.
    pub fn remove_departed(&mut self, chat: &ChatId) -> Result<Outgoing> {
        let departed = self
            .meta
            .get(chat)
            .map(|meta| meta.departed.clone())
            .unwrap_or_default();
        self.transact(|device| {
            device.remove_where(chat, |certificate| {
                departed.contains(&certificate.account())
            })
        })
    }

    fn new_chat(&mut self) -> Result<ChatId> {
        let group_id: [u8; 16] = self
            .provider
            .rand()
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a chat id", e))?;
        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &create_config(self.mailbox)?,
            GroupId::from_slice(&group_id),
            self.credential.clone(),
        )
        .map_err(|e| CoreError::mls("creating a chat", e))?;
        let chat = ChatId::of(group.group_id());
        self.chats.insert(chat.clone(), group);
        Ok(chat)
    }

    /// Adds the device behind `key_package` to `chat`, and queues the commit
    /// and the welcome in the outbox.
    ///
    /// The newcomer's device certificate and mailbox are verified first; an
    /// unverifiable or unreachable device never enters a chat.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], [`CoreError::Unauthenticated`],
    /// [`CoreError::Malformed`], [`CoreError::Mls`] or a storage error.
    pub fn invite(&mut self, chat: &ChatId, key_package: &[u8]) -> Result<Invitation> {
        self.transact(|device| {
            let key_package = device.validate_key_package(key_package)?;
            let newcomer = verify_leaf(key_package.leaf_node())?;
            device.add(chat, &[key_package], vec![newcomer])
        })
    }

    /// Certifies the device behind `applicant` for this account and seals
    /// what it needs into `offer`'s slot: the certificate, the username, the
    /// contacts and the chats this account has spoken in (ADR-0014).
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] on a device without the account key,
    /// [`CoreError::Malformed`] before the account has a username or for an
    /// applicant claiming this device's id, or a crypto error.
    pub fn approve_link(
        &mut self,
        offer: &Offer,
        applicant: &Applicant,
    ) -> Result<service::Request> {
        let account = self.account_key().ok_or(CoreError::Unauthenticated(
            "only the device holding the account key can link devices",
        ))?;
        let username = self.username.clone().ok_or(CoreError::malformed(
            "link",
            "the account has no username yet",
        ))?;
        if applicant.device == self.id() {
            return Err(CoreError::malformed(
                "link",
                "the applicant reuses this device's id",
            ));
        }
        let certificate = account.certify_device(applicant.device, &applicant.signature_key)?;
        let answer = offer.answer(&AnswerBody {
            certificate: certificate.encode(),
            username,
            contacts: self.contacts.values().cloned().collect(),
            introduced: self.introduced.iter().cloned().collect(),
            labels: self
                .labels
                .iter()
                .map(|(device, label)| (*device, label.clone()))
                .chain([(applicant.device, applicant.label.clone())])
                .collect(),
        })?;
        self.remember_label(applicant.device, &applicant.label)?;
        Ok(answer)
    }

    /// Which of this account's devices each chat still lacks, given the
    /// devices the directory lists for it ([`service::device_list`]). Every
    /// chat this account has spoken in should hold all of them (ADR-0014);
    /// chats still waiting for an answer to a commit are left for later.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a certificate of another account,
    /// or a malformed member.
    pub fn missing_devices(
        &self,
        registered: &[DeviceCertificate],
    ) -> Result<Vec<(ChatId, Vec<DeviceId>)>> {
        let own = self.account();
        if registered
            .iter()
            .any(|certificate| certificate.account() != own)
        {
            return Err(CoreError::Unauthenticated(
                "a listed device belongs to another account",
            ));
        }
        let mut missing = Vec::new();
        for chat in self.chats() {
            let group = &self.chats[&chat];
            if !self.introduced.contains(&chat) || group.pending_commit().is_some() {
                continue;
            }
            let present = group
                .members()
                .map(|member| certificate_of(&member.credential).map(|c| c.device()))
                .collect::<Result<BTreeSet<DeviceId>>>()?;
            let lacking: Vec<DeviceId> = registered
                .iter()
                .map(DeviceCertificate::device)
                .filter(|device| *device != self.id() && !present.contains(device))
                .collect();
            if !lacking.is_empty() {
                missing.push((chat.clone(), lacking));
            }
        }
        Ok(missing)
    }

    /// Which other accounts' devices each chat lacks, given the devices the
    /// directory lists for them ([`service::device_lists`]) (ADR-0018).
    ///
    /// An account heals itself while one of its devices in the chat is still
    /// registered, so only accounts with none left there count, as after a
    /// recovery. Accounts not in `lists` count as registered. Chats this
    /// account has not spoken in, or still waiting for an answer to a
    /// commit, are left for later.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a certificate of another account
    /// than its list's, or a malformed member.
    pub fn foreign_gaps(
        &self,
        lists: &[(AccountId, Vec<DeviceCertificate>)],
    ) -> Result<Vec<ForeignGap>> {
        let mut registered: BTreeMap<AccountId, BTreeSet<DeviceId>> = BTreeMap::new();
        for (account, certificates) in lists {
            if certificates
                .iter()
                .any(|certificate| certificate.account() != *account)
            {
                return Err(CoreError::Unauthenticated(
                    "a listed device belongs to another account",
                ));
            }
            registered
                .entry(*account)
                .or_default()
                .extend(certificates.iter().map(DeviceCertificate::device));
        }
        let own = self.account();
        let live = |account: &AccountId, device: &DeviceId| {
            registered
                .get(account)
                .is_none_or(|devices| devices.contains(device))
        };
        let mut gaps = Vec::new();
        for chat in self.chats() {
            let group = &self.chats[&chat];
            if !self.introduced.contains(&chat) || group.pending_commit().is_some() {
                continue;
            }
            let mut leaves = Vec::new();
            for member in group.members() {
                let certificate = certificate_of(&member.credential)?;
                leaves.push((
                    member.index.u32(),
                    certificate.account(),
                    certificate.device(),
                ));
            }
            let accounts: BTreeSet<AccountId> = leaves.iter().map(|leaf| leaf.1).collect();
            for account in accounts {
                let Some(listed) = registered.get(&account) else {
                    continue;
                };
                let still_there = leaves
                    .iter()
                    .any(|leaf| leaf.1 == account && listed.contains(&leaf.2));
                if account == own || still_there || listed.is_empty() {
                    continue;
                }
                let healer = leaves
                    .iter()
                    .filter(|leaf| leaf.1 != account && live(&leaf.1, &leaf.2))
                    .map(|leaf| leaf.0)
                    .min();
                gaps.push(ForeignGap {
                    chat: chat.clone(),
                    account,
                    devices: listed.iter().copied().collect(),
                    first: healer == Some(group.own_leaf_index().u32()),
                });
            }
        }
        Ok(gaps)
    }

    /// Which devices of this account each chat still holds although the
    /// directory no longer lists them ([`service::device_list`]): removed
    /// devices, to take out of every chat (ADR-0014). Chats still waiting
    /// for an answer to a commit are left for later.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a certificate of another account,
    /// or a malformed member.
    pub fn stale_devices(
        &self,
        registered: &[DeviceCertificate],
    ) -> Result<Vec<(ChatId, Vec<DeviceId>)>> {
        let own = self.account();
        if registered
            .iter()
            .any(|certificate| certificate.account() != own)
        {
            return Err(CoreError::Unauthenticated(
                "a listed device belongs to another account",
            ));
        }
        let listed: BTreeSet<DeviceId> = registered.iter().map(DeviceCertificate::device).collect();
        let mut stale = Vec::new();
        for chat in self.chats() {
            let group = &self.chats[&chat];
            if group.pending_commit().is_some() {
                continue;
            }
            let mut gone = Vec::new();
            for member in group.members() {
                let certificate = certificate_of(&member.credential)?;
                let device = certificate.device();
                if certificate.account() == own && device != self.id() && !listed.contains(&device)
                {
                    gone.push(device);
                }
            }
            if !gone.is_empty() {
                stale.push((chat, gone));
            }
        }
        Ok(stale)
    }

    /// Takes `devices`, all of this account, out of `chat` in one commit.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], [`CoreError::Malformed`] when none of
    /// them is in the chat or one is this device, or an MLS or storage
    /// error.
    pub fn remove_own_devices(&mut self, chat: &ChatId, devices: &[DeviceId]) -> Result<Outgoing> {
        if devices.contains(&self.id()) {
            return Err(CoreError::malformed(
                "removal",
                "a device cannot remove itself",
            ));
        }
        let own = self.account();
        self.transact(|device| {
            device.remove_where(chat, |certificate| {
                certificate.account() == own && devices.contains(&certificate.device())
            })
        })
    }

    /// Takes every member `chosen` picks out of `chat` in one commit.
    fn remove_where(
        &mut self,
        chat: &ChatId,
        chosen: impl Fn(&DeviceCertificate) -> bool,
    ) -> Result<Outgoing> {
        let group = self.chats.get(chat).ok_or(CoreError::UnknownChat)?;
        let mut leaves = Vec::new();
        for member in group.members() {
            if chosen(&certificate_of(&member.credential)?) {
                leaves.push(member.index);
            }
        }
        if leaves.is_empty() || leaves.contains(&group.own_leaf_index()) {
            return Err(CoreError::malformed("removal", "no one to take out"));
        }
        let group = self.chats.get_mut(chat).ok_or(CoreError::UnknownChat)?;
        // The removed devices get the commit too: it tells them they are
        // out.
        let everyone = recipients(group)?;
        let epoch = group.epoch().as_u64();
        let (commit, _welcome, _group_info) = group
            .remove_members(&self.provider, &self.signer, &leaves)
            .map_err(|e| CoreError::mls("removing a device", e))?;
        let commit = Outgoing {
            id: EnvelopeId::random()?,
            recipients: everyone,
            payload: encode(&commit, "encoding a commit")?,
            commit: None,
            urgent: false,
        };
        self.queue_commit(chat, epoch, commit, None)
    }

    /// Adds this account's own devices to `chat` in one commit, each from a
    /// directory answer about that device alone ([`service::device_lookup`]).
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a device of another account, or as
    /// [`Device::invite`].
    pub fn add_own_devices(
        &mut self,
        chat: &ChatId,
        entries: &[DirectoryEntry],
    ) -> Result<Invitation> {
        self.add_devices(chat, self.account(), entries)
    }

    /// Adds devices of `account`, which already has a device in `chat`, in
    /// one commit, each from a directory answer about that device alone:
    /// one's own new devices, or another account's after a recovery
    /// ([`Device::foreign_gaps`]).
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a device of another account,
    /// [`CoreError::Malformed`] when `account` is not in the chat, or as
    /// [`Device::invite`].
    pub fn add_devices(
        &mut self,
        chat: &ChatId,
        account: AccountId,
        entries: &[DirectoryEntry],
    ) -> Result<Invitation> {
        let group = self.chats.get(chat).ok_or(CoreError::UnknownChat)?;
        let mut in_chat = false;
        for member in group.members() {
            in_chat |= certificate_of(&member.credential)?.account() == account;
        }
        if !in_chat {
            return Err(CoreError::malformed(
                "device lookup",
                "the account is not in the chat",
            ));
        }
        if entries.is_empty() {
            return Err(CoreError::malformed("device lookup", "no device to add"));
        }
        self.transact(|device| {
            let mut packages = Vec::with_capacity(entries.len());
            let mut newcomers = Vec::with_capacity(entries.len());
            for entry in entries {
                let [bytes] = entry.key_packages.as_slice() else {
                    return Err(CoreError::malformed(
                        "device lookup",
                        "not exactly one device",
                    ));
                };
                let key_package = device.validate_key_package(bytes)?;
                let leaf = key_package.leaf_node();
                if entry.account != account
                    || certificate_of(leaf.credential())?.account() != account
                {
                    return Err(CoreError::Unauthenticated("not a device of this account"));
                }
                newcomers.push(verify_leaf(leaf)?);
                packages.push(key_package);
            }
            device.add(chat, &packages, newcomers)
        })
    }

    fn validate_key_package(&self, bytes: &[u8]) -> Result<KeyPackage> {
        let key_package = KeyPackageIn::tls_deserialize_exact(bytes)
            .map_err(|_| CoreError::malformed("key package", "invalid encoding"))?
            .validate(self.provider.crypto(), ProtocolVersion::Mls10)
            .map_err(|e| CoreError::mls("validating a key package", e))?;
        if key_package.ciphersuite() != CIPHERSUITE {
            return Err(CoreError::malformed(
                "key package",
                "unsupported ciphersuite",
            ));
        }
        Ok(key_package)
    }

    /// Adds verified devices to `chat` in one commit and queues the commit
    /// and the welcome.
    fn add(
        &mut self,
        chat: &ChatId,
        packages: &[KeyPackage],
        mut newcomers: Vec<MailboxId>,
    ) -> Result<Invitation> {
        let group = self.chats.get_mut(chat).ok_or(CoreError::UnknownChat)?;
        let existing = recipients(group)?;
        let epoch = group.epoch().as_u64();
        let (commit, welcome, _group_info) = group
            .add_members(&self.provider, &self.signer, packages)
            .map_err(|e| CoreError::mls("adding a member", e))?;
        newcomers.sort_unstable();
        newcomers.dedup();
        let welcome = Outgoing {
            id: EnvelopeId::random()?,
            recipients: newcomers,
            payload: encode(&welcome, "encoding a welcome")?,
            commit: None,
            urgent: false,
        };
        let commit = Outgoing {
            id: EnvelopeId::random()?,
            recipients: existing,
            payload: encode(&commit, "encoding a commit")?,
            commit: None,
            urgent: false,
        };
        let commit = self.queue_commit(chat, epoch, commit, Some(welcome.clone()))?;
        Ok(Invitation { commit, welcome })
    }

    /// Sends `commit`, made in `epoch`. With nobody else in `chat` nothing
    /// can race it: it applies at once and its welcome goes out as is.
    /// Otherwise it goes through the sequencer and applies here once it
    /// wins ([`Device::settle`]).
    fn queue_commit(
        &mut self,
        chat: &ChatId,
        epoch: u64,
        mut commit: Outgoing,
        welcome: Option<Outgoing>,
    ) -> Result<Outgoing> {
        if commit.recipients.is_empty() {
            self.chats
                .get_mut(chat)
                .ok_or(CoreError::UnknownChat)?
                .merge_pending_commit(&self.provider)
                .map_err(|e| CoreError::mls("applying a change to a chat", e))?;
            if let Some(welcome) = &welcome {
                self.queue(welcome, None)?;
            }
        } else {
            commit.commit = Some(CommitTicket {
                group: chat.0.clone(),
                epoch,
                welcome: welcome.map(Box::new),
            });
            self.queue(&commit, None)?;
        }
        Ok(commit)
    }

    /// Joins a chat from a welcome addressed to this device.
    ///
    /// Every member's device certificate and mailbox are verified before the
    /// chat is accepted.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`], [`CoreError::Unauthenticated`],
    /// [`CoreError::Mls`] or a storage error.
    pub fn join(&mut self, welcome: &[u8]) -> Result<ChatId> {
        self.transact(|device| {
            let message = MlsMessageIn::tls_deserialize_exact(welcome)
                .map_err(|_| CoreError::malformed("welcome", "invalid encoding"))?;
            let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
                return Err(CoreError::malformed("welcome", "not a welcome message"));
            };
            device.join_welcome(welcome).map(|(chat, _)| chat)
        })
    }

    fn join_welcome(&mut self, welcome: Welcome) -> Result<(ChatId, Author)> {
        let staged = StagedWelcome::new_from_welcome(&self.provider, &join_config(), welcome, None)
            .map_err(|e| CoreError::mls("reading a welcome", e))?;
        let chat = ChatId::of(staged.group_context().group_id());
        if self.chats.contains_key(&chat) {
            return Err(CoreError::malformed("welcome", "already in this chat"));
        }
        let inviter = certificate_of(
            staged
                .welcome_sender()
                .map_err(|e| CoreError::mls("reading a welcome", e))?
                .credential(),
        )?;
        let group = staged
            .into_group(&self.provider)
            .map_err(|e| CoreError::mls("joining a chat", e))?;
        // Verifies the inviter too. A refusal rolls the stored group back
        // with everything else.
        verify_roster(&group)?;
        self.chats.insert(chat.clone(), group);
        Ok((
            chat,
            Author {
                account: inviter.account(),
                device: inviter.device(),
            },
        ))
    }

    /// Encrypts `message` for everyone else in `chat`, records it in history
    /// and queues it in the outbox, all in one transaction.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], [`CoreError::TooLarge`],
    /// [`CoreError::Malformed`] (a message id already used in this chat),
    /// [`CoreError::Mls`] or a storage error.
    pub fn send(&mut self, chat: &ChatId, message: &AppMessage) -> Result<Outgoing> {
        let plaintext = message.encode()?;
        self.transact(|device| {
            device.introduce(chat)?;
            device.send_now(chat, message, &plaintext)
        })
    }

    fn send_now(
        &mut self,
        chat: &ChatId,
        message: &AppMessage,
        plaintext: &[u8],
    ) -> Result<Outgoing> {
        let group = self.chats.get_mut(chat).ok_or(CoreError::UnknownChat)?;
        let others = recipients(group)?;
        let out = group
            .create_message(&self.provider, &self.signer, plaintext)
            .map_err(|e| CoreError::mls("encrypting a message", e))?;
        let outgoing = Outgoing {
            id: EnvelopeId::random()?,
            recipients: others,
            payload: encode(&out, "encoding a message")?,
            commit: None,
            urgent: matches!(message.body, Body::Call { .. }),
        };
        // Alone in a chat, there is no one to deliver to: already "sent".
        let status = if outgoing.recipients.is_empty() {
            SendStatus::Sent
        } else {
            SendStatus::Queued
        };
        self.fold(chat, self.author(), message, status)?;
        let tracked = history::inserts(&message.body).then(|| (chat.clone(), message.id));
        self.queue(&outgoing, tracked)?;
        Ok(outgoing)
    }

    /// Sends a receipt for `messages` into `chat`.
    fn acknowledge(
        &mut self,
        chat: &ChatId,
        status: ReceiptStatus,
        messages: Vec<MessageId>,
    ) -> Result<()> {
        let receipt = AppMessage::new(
            MessageId::random()?,
            now_ms(),
            Body::Receipt { status, messages },
        );
        self.send_now(chat, &receipt, &receipt.encode()?)?;
        Ok(())
    }

    /// Accepts a chat request: introduces this device there, so the people
    /// in the chat learn its username and, from now on, get receipts.
    /// Writing in the chat accepts it too.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or a crypto, MLS or storage error.
    pub fn accept_chat(&mut self, chat: &ChatId) -> Result<()> {
        if !self.chats.contains_key(chat) {
            return Err(CoreError::UnknownChat);
        }
        self.transact(|device| device.introduce(chat))
    }

    /// Removes `chat` from this device: its keys, its history, its marks.
    /// Nobody is told; whatever still arrives for it is refused as a
    /// message for an unknown chat. This is how a request is declined.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or an MLS or storage error.
    pub fn delete_chat(&mut self, chat: &ChatId) -> Result<()> {
        self.transact(|device| device.erase(chat))
    }

    /// Erases `chat` from this device, as part of the operation in progress.
    fn erase(&mut self, chat: &ChatId) -> Result<()> {
        let mut group = self.chats.remove(chat).ok_or(CoreError::UnknownChat)?;
        group
            .delete(self.provider.storage())
            .map_err(|e| CoreError::mls("erasing a chat", e))?;
        self.introduced.remove(chat);
        self.read_marks.remove(chat);
        self.meta.remove(chat);
        self.pending.erased.push(chat.clone());
        Ok(())
    }

    /// Marks everything in `chat` as read here. In a chat the user has
    /// written in, the authors get a read receipt for their messages (the
    /// newest 256 at most). Returns how many messages that covered.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or a crypto, MLS or storage error.
    pub fn mark_read(&mut self, chat: &ChatId) -> Result<usize> {
        let mark = self.read_marks.get(chat).copied().unwrap_or(0);
        let newest = self.history(chat, None, u32::try_from(MAX_RECEIPT_IDS).unwrap_or(256))?;
        let Some(top) = newest.first().map(|entry| entry.seq) else {
            return Ok(0);
        };
        if top <= mark {
            return Ok(0);
        }
        let unread: Vec<MessageId> = newest
            .iter()
            .take_while(|entry| entry.seq > mark)
            .filter(|entry| unread_message(&entry.message))
            .map(|entry| entry.message.id)
            .collect();
        let count = unread.len();
        self.transact(|device| {
            if device.introduced.contains(chat) && !unread.is_empty() {
                device.acknowledge(chat, ReceiptStatus::Read, unread)?;
            }
            device.read_marks.insert(chat.clone(), top);
            Ok(())
        })?;
        Ok(count)
    }

    /// Messages from others in `chat` past its read mark, up to the cap.
    fn unread(&self, chat: &ChatId) -> Result<u32> {
        let mark = self.read_marks.get(chat).copied().unwrap_or(0);
        let count = self
            .history(chat, None, UNREAD_CAP + 1)?
            .iter()
            .take_while(|entry| entry.seq > mark)
            .filter(|entry| unread_message(&entry.message))
            .count();
        Ok(u32::try_from(count).unwrap_or(UNREAD_CAP).min(UNREAD_CAP))
    }

    /// How many other accounts are in `chat`: the people a message there
    /// must reach before it counts as delivered to everyone.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], or a malformed member.
    pub fn others_in(&self, chat: &ChatId) -> Result<usize> {
        let own = self.account();
        let group = self.chats.get(chat).ok_or(CoreError::UnknownChat)?;
        let accounts = group
            .members()
            .map(|member| certificate_of(&member.credential).map(|c| c.account()))
            .collect::<Result<BTreeSet<AccountId>>>()?;
        Ok(accounts.iter().filter(|account| **account != own).count())
    }

    /// Sends this device's profile into `chat`, once, before its first
    /// message there, so the others learn which username it goes by.
    fn introduce(&mut self, chat: &ChatId) -> Result<()> {
        let Some(username) = self.username.clone() else {
            return Ok(());
        };
        if self.introduced.contains(chat) {
            return Ok(());
        }
        let profile = AppMessage::new(MessageId::random()?, now_ms(), Body::Profile { username });
        self.send_now(chat, &profile, &profile.encode()?)?;
        self.introduced.insert(chat.clone());
        Ok(())
    }

    /// Decrypts and authenticates an incoming MLS message, and folds it into
    /// the chat's history. A welcome joins the chat it invites to.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`], [`CoreError::UnknownChat`],
    /// [`CoreError::Unauthenticated`], [`CoreError::Mls`] or a storage error.
    /// A message that fails here must be dropped, never shown; the device's
    /// state is unchanged.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Received> {
        self.transact(|device| device.receive_inner(bytes))
    }

    fn receive_inner(&mut self, bytes: &[u8]) -> Result<Received> {
        let message = MlsMessageIn::tls_deserialize_exact(bytes)
            .map_err(|_| CoreError::malformed("MLS message", "invalid encoding"))?;
        let protocol: ProtocolMessage = match message.extract() {
            MlsMessageBodyIn::Welcome(welcome) => {
                let (chat, by) = self.join_welcome(welcome)?;
                return Ok(Received::Joined { chat, by });
            }
            MlsMessageBodyIn::PrivateMessage(message) => message.into(),
            MlsMessageBodyIn::PublicMessage(message) => message.into(),
            _ => return Err(CoreError::malformed("MLS message", "not a chat message")),
        };
        let chat = ChatId::of(protocol.group_id());
        if protocol.content_type() == ContentType::Commit {
            self.resolve_pending_commit(&chat, protocol.epoch().as_u64())?;
        }
        let group = self.chats.get_mut(&chat).ok_or(CoreError::UnknownChat)?;
        let processed = group
            .process_message(&self.provider, protocol)
            .map_err(|e| CoreError::mls("processing a message", e))?;

        let sender = processed.credential().clone();
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(application) => {
                let certificate = certificate_of(&sender)?;
                let message = AppMessage::decode(&application.into_bytes())?;
                let author = Author {
                    account: certificate.account(),
                    device: certificate.device(),
                };
                self.fold(&chat, author, &message, SendStatus::Sent)?;
                if author.account == self.account() {
                    // Another of our devices spoke here: the chat is ours on
                    // this device too (ADR-0014).
                    self.introduced.insert(chat.clone());
                }
                if message.body == Body::Leave {
                    if author.account == self.account() {
                        // Another of our devices left: so does this one.
                        self.erase(&chat)?;
                    } else {
                        self.meta
                            .entry(chat.clone())
                            .or_default()
                            .departed
                            .insert(author.account);
                    }
                }
                if let Body::Profile { username } = &message.body
                    && author.account != self.account()
                {
                    contacts::claimed(&mut self.contacts, author.account, username);
                }
                if history::inserts(&message.body)
                    && author.account != self.account()
                    && self.introduced.contains(&chat)
                {
                    self.acknowledge(&chat, ReceiptStatus::Delivered, vec![message.id])?;
                }
                Ok(Received::Message {
                    chat,
                    author,
                    message,
                })
            }
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                // Every leaf a commit brings in must be a verified, reachable
                // device: added members, updated leaves and the committer's
                // own new leaf.
                for added in staged.add_proposals() {
                    let leaf = added.add_proposal().key_package().leaf_node();
                    verify_leaf(leaf)?;
                    // Someone who left and is added again is a member again.
                    let account = certificate_of(leaf.credential())?.account();
                    if let Some(meta) = self.meta.get_mut(&chat) {
                        meta.departed.remove(&account);
                    }
                }
                for updated in staged.update_proposals() {
                    verify_leaf(updated.update_proposal().leaf_node())?;
                }
                if let Some(leaf) = staged.update_path_leaf_node() {
                    verify_leaf(leaf)?;
                }
                group
                    .merge_staged_commit(&self.provider, *staged)
                    .map_err(|e| CoreError::mls("applying a chat update", e))?;
                Ok(if group.is_active() {
                    Received::ChatUpdated { chat }
                } else {
                    Received::Left { chat }
                })
            }
            // Zelamar merges its own commits immediately and frames everything
            // as PrivateMessage, so an echo of our own traffic is a duplicate.
            ProcessedMessageContent::OwnPrivateMessage
            | ProcessedMessageContent::OwnPendingCommit => Ok(Received::OwnEcho { chat }),
            ProcessedMessageContent::ProposalMessage(_)
            | ProcessedMessageContent::ExternalJoinProposalMessage(_) => Err(CoreError::malformed(
                "MLS message",
                "standalone proposals are not used by Zelamar",
            )),
        }
    }

    /// Another member's commit for `epoch` arrived while one of ours waits
    /// for the sequencer. Every commit that reaches a mailbox has won its
    /// epoch, so: for our own epoch, ours lost and is dropped; for the next
    /// one, ours must have won (that commit builds on it) and applies first.
    fn resolve_pending_commit(&mut self, chat: &ChatId, epoch: u64) -> Result<()> {
        let Some(group) = self.chats.get_mut(chat) else {
            return Ok(());
        };
        if group.pending_commit().is_none() {
            return Ok(());
        }
        let ours = group.epoch().as_u64();
        if epoch == ours {
            group
                .clear_pending_commit(self.provider.storage())
                .map_err(|e| CoreError::mls("dropping a lost commit", e))?;
            for bytes in self.store.outbox()? {
                let item = Queued::decode(&bytes)?;
                if item
                    .outgoing
                    .commit
                    .as_ref()
                    .is_some_and(|ticket| ticket.group == chat.0 && ticket.epoch == ours)
                {
                    self.pending.confirmed.push(item.outgoing.id);
                }
            }
        } else if epoch == ours + 1 {
            group
                .merge_pending_commit(&self.provider)
                .map_err(|e| CoreError::mls("applying a commit that won", e))?;
        }
        Ok(())
    }

    const fn author(&self) -> Author {
        Author {
            account: self.account(),
            device: self.id(),
        }
    }

    /// Folds `message` into the history of `chat`, as part of the operation
    /// in progress. New records start with `status`.
    fn fold(
        &mut self,
        chat: &ChatId,
        author: Author,
        message: &AppMessage,
        status: SendStatus,
    ) -> Result<()> {
        if let Body::Name { name } = &message.body
            && self.chat_name(chat) == Some(name.as_str())
        {
            // The same name again, sent for people who just joined: nothing
            // to show to those who knew it.
            self.note_name(chat, message, name);
            return Ok(());
        }
        let records = history::apply(
            &ChatHistory {
                store: &self.store,
                chat,
                pending: &self.pending.messages,
            },
            author,
            author.account == self.account(),
            message,
            now_ms(),
            status,
        )?;
        self.pending
            .messages
            .extend(records.into_iter().map(|record| (chat.clone(), record)));
        if let Body::Name { name } = &message.body {
            self.note_name(chat, message, name);
        }
        Ok(())
    }

    /// Keeps `name` as the chat's name if `message` is the newest naming.
    fn note_name(&mut self, chat: &ChatId, message: &AppMessage, name: &str) {
        let meta = self.meta.entry(chat.clone()).or_default();
        let newer = meta
            .name
            .as_ref()
            .is_none_or(|(at, id, _)| (message.sent_at_ms, message.id.0) > (*at, id.0));
        if newer {
            meta.name = Some((message.sent_at_ms, message.id, name.to_owned()));
        }
    }

    /// Tells people who just joined `chat` its name, if it has one.
    fn announce_name(&mut self, chat: &ChatId) -> Result<()> {
        let Some(name) = self.chat_name(chat).map(str::to_owned) else {
            return Ok(());
        };
        let message = AppMessage::new(MessageId::random()?, now_ms(), Body::Name { name });
        self.send_now(chat, &message, &message.encode()?)?;
        Ok(())
    }

    /// Queues `outgoing` in the outbox, as part of the operation in progress.
    /// With no recipients there is nothing to deliver, so nothing is queued.
    fn queue(&mut self, outgoing: &Outgoing, message: Option<(ChatId, MessageId)>) -> Result<()> {
        if outgoing.recipients.is_empty() && outgoing.commit.is_none() {
            return Ok(());
        }
        let item = Queued {
            outgoing: outgoing.clone(),
            message,
        }
        .encode()?;
        self.pending.outbox.push((outgoing.id, item));
        Ok(())
    }

    /// Runs `op` as a unit of work: persisted on success, rolled back on
    /// failure.
    fn transact<T>(&mut self, op: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        match op(self) {
            Ok(value) => match self.persist() {
                Ok(()) => Ok(value),
                Err(error) => {
                    self.rollback()?;
                    Err(error)
                }
            },
            Err(error) => {
                self.rollback()?;
                Err(error)
            }
        }
    }

    /// Writes every MLS storage change since the last persist, the chat list
    /// if it changed, and the pending history and outbox writes, in one
    /// transaction.
    fn persist(&mut self) -> Result<()> {
        self.persist_with(Vec::new())
    }

    /// As [`Self::persist`], with `records` written in the same transaction.
    fn persist_with(&mut self, records: Vec<(&'static str, Vec<u8>)>) -> Result<()> {
        let mut batch = Batch {
            records,
            ..Batch::default()
        };
        {
            let current = self
                .provider
                .storage()
                .values
                .read()
                .map_err(|_| poisoned())?;
            for (key, value) in current.iter() {
                if self.persisted.get(key) != Some(value) {
                    batch.mls_upserts.push((key.clone(), value.clone()));
                }
            }
            for key in self.persisted.keys() {
                if !current.contains_key(key) {
                    batch.mls_deletes.push(key.clone());
                }
            }
        }
        let chats: HashSet<ChatId> = self.chats.keys().cloned().collect();
        if chats != self.persisted_chats {
            batch.records.push((RECORD_CHATS, encode_chat_list(&chats)));
        }
        if self.contacts != self.persisted_contacts {
            batch
                .records
                .push((RECORD_CONTACTS, contacts::encode(&self.contacts)?));
        }
        if self.read_marks != self.persisted_read_marks {
            batch
                .records
                .push((RECORD_READ_MARKS, encode_read_marks(&self.read_marks)?));
        }
        if self.meta != self.persisted_meta {
            batch
                .records
                .push((RECORD_CHAT_META, encode_meta(&self.meta)?));
        }
        if self.introduced != self.persisted_introduced {
            let introduced: HashSet<ChatId> = self.introduced.iter().cloned().collect();
            batch
                .records
                .push((RECORD_INTRODUCED, encode_chat_list(&introduced)));
        }
        let pending = std::mem::take(&mut self.pending);
        for (chat, record) in &pending.messages {
            batch
                .messages
                .push((chat.0.clone(), record.id.0, record.encode()?));
        }
        batch.outbox_inserts = pending
            .outbox
            .into_iter()
            .map(|(id, item)| (id.0, item))
            .collect();
        batch.outbox_deletes = pending.confirmed.iter().map(|id| id.0).collect();
        batch.chat_deletes = pending.erased.iter().map(|chat| chat.0.clone()).collect();
        if batch.is_empty() {
            return Ok(());
        }

        self.store.apply(&batch)?;
        for (key, value) in batch.mls_upserts {
            self.persisted.insert(key, value);
        }
        for key in &batch.mls_deletes {
            self.persisted.remove(key);
        }
        self.persisted_chats = chats;
        self.persisted_contacts.clone_from(&self.contacts);
        self.persisted_introduced.clone_from(&self.introduced);
        self.persisted_read_marks.clone_from(&self.read_marks);
        self.persisted_meta.clone_from(&self.meta);
        Ok(())
    }

    /// Restores the in-memory state to exactly what is on disk.
    fn rollback(&mut self) -> Result<()> {
        self.provider
            .storage()
            .values
            .write()
            .map_err(|_| poisoned())?
            .clone_from(&self.persisted);
        self.chats = load_chats(&self.provider, &self.persisted_chats)?;
        self.pending = Pending::default();
        self.contacts.clone_from(&self.persisted_contacts);
        self.introduced.clone_from(&self.persisted_introduced);
        self.read_marks.clone_from(&self.persisted_read_marks);
        self.meta.clone_from(&self.persisted_meta);
        Ok(())
    }
}

/// History of one chat as the operation in progress sees it: its own pending
/// writes first, then the store.
struct ChatHistory<'a> {
    store: &'a Store,
    chat: &'a ChatId,
    pending: &'a [(ChatId, StoredMessage)],
}

impl HistoryReader for ChatHistory<'_> {
    fn load(&self, id: MessageId) -> Result<Option<StoredMessage>> {
        let latest = self
            .pending
            .iter()
            .rev()
            .find(|(chat, record)| chat == self.chat && record.id == id);
        if let Some((_, record)) = latest {
            return Ok(Some(record.clone()));
        }
        self.store
            .message(self.chat.as_bytes(), &id.0)?
            .map(|bytes| StoredMessage::decode(&bytes))
            .transpose()
    }
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("account", &self.account())
            .field("id", &self.id())
            .field("chats", &self.chats.len())
            .finish_non_exhaustive()
    }
}

fn create_config(mailbox: MailboxId) -> Result<MlsGroupCreateConfig> {
    Ok(MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .capabilities(capabilities())
        .with_leaf_node_extensions(mailbox_extension(mailbox)?)
        .map_err(|e| CoreError::mls("publishing the mailbox", e))?
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(PADDING)
        .max_past_epochs(MAX_PAST_EPOCHS)
        .build())
}

fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(PADDING)
        .max_past_epochs(MAX_PAST_EPOCHS)
        .build()
}

/// What every Zelamar leaf supports: the one ciphersuite, basic
/// credentials, the mailbox extension and last-resort key packages.
fn capabilities() -> Capabilities {
    Capabilities::new(
        None,
        Some(&[CIPHERSUITE]),
        Some(&[
            ExtensionType::Unknown(MAILBOX_EXTENSION),
            ExtensionType::LastResort,
        ]),
        None,
        Some(&[CredentialType::Basic]),
    )
}

fn mailbox_extension(mailbox: MailboxId) -> Result<Extensions<LeafNode>> {
    Extensions::single(Extension::Unknown(
        MAILBOX_EXTENSION,
        UnknownExtension(mailbox.0.to_vec()),
    ))
    .map_err(|e| CoreError::mls("publishing the mailbox", e))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn credential_for(certificate: &DeviceCertificate, signer: &SignatureKeyPair) -> CredentialWithKey {
    CredentialWithKey {
        credential: BasicCredential::new(certificate.encode()).into(),
        signature_key: signer.public().into(),
    }
}

fn load_chats(
    provider: &OpenMlsRustCrypto,
    ids: &HashSet<ChatId>,
) -> Result<HashMap<ChatId, MlsGroup>> {
    ids.iter()
        .map(|chat| {
            let group = MlsGroup::load(provider.storage(), &GroupId::from_slice(chat.as_bytes()))
                .map_err(|e| CoreError::mls("loading a chat", e))?
                .ok_or(CoreError::malformed("store", "a listed chat has no state"))?;
            Ok((chat.clone(), group))
        })
        .collect()
}

/// Read marks record: CBOR list of `(chat, seq)`.
fn encode_read_marks(marks: &BTreeMap<ChatId, i64>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::into_writer(&marks.iter().collect::<Vec<_>>(), &mut out).map_err(|e| {
        CoreError::Encoding {
            what: "read marks",
            source: e.into(),
        }
    })?;
    Ok(out)
}

fn encode_labels(labels: &BTreeMap<DeviceId, String>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::into_writer(labels, &mut out).map_err(|e| CoreError::Encoding {
        what: "device labels",
        source: e.into(),
    })?;
    Ok(out)
}

/// Whether `message` is someone else's message the user has not seen:
/// events (a name, someone leaving) are lines, not messages, and a call
/// someone picked up was not missed.
const fn unread_message(message: &StoredMessage) -> bool {
    !message.outgoing
        && !message.deleted
        && !message.content.is_event()
        && !message.content.is_answered_call()
}

/// The record `name` decoded, or the empty value if it was never written.
fn decoded_or_default<T: Default>(
    store: &Store,
    name: &str,
    decode: impl Fn(&[u8]) -> Result<T>,
) -> Result<T> {
    store
        .record(name)?
        .map(|bytes| decode(&bytes))
        .transpose()
        .map(Option::unwrap_or_default)
}

fn encode_meta(meta: &BTreeMap<ChatId, ChatMeta>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::into_writer(&meta.iter().collect::<Vec<_>>(), &mut out).map_err(|e| {
        CoreError::Encoding {
            what: "chat details",
            source: e.into(),
        }
    })?;
    Ok(out)
}

fn decode_meta(bytes: &[u8]) -> Result<BTreeMap<ChatId, ChatMeta>> {
    let list: Vec<(ChatId, ChatMeta)> = ciborium::from_reader(bytes)
        .map_err(|_| CoreError::malformed("store", "chat details are malformed"))?;
    Ok(list.into_iter().collect())
}

fn encode_recovery(record: &RecoveryRecord) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::into_writer(record, &mut out).map_err(|e| CoreError::Encoding {
        what: "recovery key",
        source: e.into(),
    })?;
    Ok(out)
}

fn decode_recovery(bytes: &[u8]) -> Result<RecoveryRecord> {
    ciborium::from_reader(bytes)
        .map_err(|_| CoreError::malformed("store", "recovery key is malformed"))
}

fn decode_labels(bytes: &[u8]) -> Result<BTreeMap<DeviceId, String>> {
    ciborium::from_reader(bytes)
        .map_err(|_| CoreError::malformed("store", "device labels are malformed"))
}

fn decode_read_marks(bytes: &[u8]) -> Result<BTreeMap<ChatId, i64>> {
    let list: Vec<(ChatId, i64)> = ciborium::from_reader(bytes)
        .map_err(|_| CoreError::malformed("read marks", "invalid encoding"))?;
    Ok(list.into_iter().collect())
}

/// Chat list record: a sequence of `u8 length || id`.
fn encode_chat_list(chats: &HashSet<ChatId>) -> Vec<u8> {
    let mut ids: Vec<&ChatId> = chats.iter().collect();
    ids.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for chat in ids {
        let len = u8::try_from(chat.0.len()).unwrap_or(u8::MAX);
        out.push(len);
        out.extend_from_slice(&chat.0[..usize::from(len)]);
    }
    out
}

fn decode_chat_list(bytes: &[u8]) -> Result<HashSet<ChatId>> {
    let mut chats = HashSet::new();
    let mut rest = bytes;
    while let Some((&len, tail)) = rest.split_first() {
        let len = usize::from(len);
        if tail.len() < len {
            return Err(CoreError::malformed("chat list", "truncated"));
        }
        let (id, tail) = tail.split_at(len);
        chats.insert(ChatId(id.to_vec()));
        rest = tail;
    }
    Ok(chats)
}

fn poisoned() -> CoreError {
    CoreError::storage("MLS state lock", "poisoned by an earlier panic")
}

fn encode(message: &MlsMessageOut, context: &'static str) -> Result<Vec<u8>> {
    message
        .tls_serialize_detached()
        .map_err(|e| CoreError::mls(context, e))
}

/// Extracts the device certificate carried in an MLS credential.
fn certificate_of(credential: &Credential) -> Result<DeviceCertificate> {
    let basic = BasicCredential::try_from(credential.clone())
        .map_err(|_| CoreError::Unauthenticated("member credential is not a Zelamar credential"))?;
    DeviceCertificate::decode(basic.identity())
}

/// The mailbox a member publishes in its leaf.
fn mailbox_of(leaf: &LeafNode) -> Result<MailboxId> {
    let extension = leaf
        .extensions()
        .unknown(MAILBOX_EXTENSION)
        .ok_or(CoreError::malformed("member", "publishes no mailbox"))?;
    extension
        .0
        .as_slice()
        .try_into()
        .map(MailboxId)
        .map_err(|_| CoreError::malformed("member", "mailbox is not 32 bytes"))
}

/// Accepts a member leaf only if its certificate verifies, certifies the
/// leaf's signature key, and the leaf publishes a mailbox. Returns that
/// mailbox.
fn verify_leaf(leaf: &LeafNode) -> Result<MailboxId> {
    verify_member(leaf.credential(), leaf.signature_key().as_slice())?;
    mailbox_of(leaf)
}

/// Verifies every member of `group`, this device included.
fn verify_roster(group: &MlsGroup) -> Result<()> {
    for member in group.members() {
        verify_leaf(leaf_at(group, member.index)?)?;
    }
    Ok(())
}

/// The mailboxes of every member of `group` except this device, sorted and
/// without duplicates.
fn recipients(group: &MlsGroup) -> Result<Vec<MailboxId>> {
    let own = group.own_leaf_index();
    let mut mailboxes = group
        .members()
        .filter(|member| member.index != own)
        .map(|member| mailbox_of(leaf_at(group, member.index)?))
        .collect::<Result<Vec<_>>>()?;
    mailboxes.sort_unstable();
    mailboxes.dedup();
    Ok(mailboxes)
}

fn leaf_at(group: &MlsGroup, index: LeafNodeIndex) -> Result<&LeafNode> {
    group
        .public_group()
        .leaf(index)
        .ok_or(CoreError::malformed("chat", "a member has no leaf"))
}

/// Accepts a member only if its certificate verifies **and** certifies the
/// exact signature key the member uses in MLS.
fn verify_member(credential: &Credential, signature_key: &[u8]) -> Result<()> {
    let certificate = certificate_of(credential)?;
    certificate.verify()?;
    if certificate.signature_key() != signature_key {
        return Err(CoreError::Unauthenticated(
            "member signature key is not the certified device key",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_device(account: &AccountKey) -> (tempfile::TempDir, Device) {
        let dir = tempfile::tempdir().expect("temp dir");
        let master = MasterKey::generate().expect("master key");
        let device =
            Device::create(&dir.path().join("device.db"), &master, account).expect("device");
        (dir, device)
    }

    /// The Flutter bridge shares devices across threads.
    #[test]
    fn device_is_send_and_sync() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Device>();
    }

    #[test]
    fn chats_use_the_post_quantum_ciphersuite() {
        let account = AccountKey::generate().expect("account");
        let (_dir, mut device) = temp_device(&account);
        let chat = device.create_chat().expect("chat");
        assert_eq!(device.chats[&chat].ciphersuite(), CIPHERSUITE);
    }

    /// Key substitution: Mallory presents Alice's genuine certificate but
    /// her own signature key. The invite must be refused.
    #[test]
    fn a_borrowed_certificate_is_refused() {
        let alice_account = AccountKey::generate().expect("account");
        let (_a, mut alice) = temp_device(&alice_account);
        let mallory_account = AccountKey::generate().expect("account");
        let (_m, mut mallory) = temp_device(&mallory_account);

        mallory.credential.credential = alice.credential.credential.clone();
        let key_package = mallory.key_package().expect("key package");

        let chat = alice.create_chat().expect("chat");
        assert!(matches!(
            alice.invite(&chat, &key_package),
            Err(CoreError::Unauthenticated(_))
        ));
    }

    /// A certificate signed by the wrong account key never verifies.
    #[test]
    fn a_self_issued_certificate_for_someone_else_is_refused() {
        let alice_account = AccountKey::generate().expect("account");
        let mallory_account = AccountKey::generate().expect("account");
        let (_m, mut mallory) = temp_device(&mallory_account);

        // Mallory re-labels her certificate as Alice's account.
        let mut forged = mallory.certificate.encode();
        forged[1..33].copy_from_slice(alice_account.id().as_bytes());
        mallory.credential.credential = BasicCredential::new(forged).into();
        let key_package = mallory.key_package().expect("key package");

        let (_a, mut alice) = temp_device(&alice_account);
        let chat = alice.create_chat().expect("chat");
        assert!(matches!(
            alice.invite(&chat, &key_package),
            Err(CoreError::Unauthenticated(_))
        ));
    }

    /// An unreachable device is a broken member: a key package without a
    /// mailbox is refused, and the refusal leaves nothing behind.
    #[test]
    fn a_key_package_without_a_mailbox_is_refused() {
        let (_m, mallory) = temp_device(&AccountKey::generate().expect("account"));
        let bundle = KeyPackage::builder()
            .build(
                CIPHERSUITE,
                &mallory.provider,
                &mallory.signer,
                mallory.credential.clone(),
            )
            .expect("key package");
        let key_package = bundle
            .key_package()
            .tls_serialize_detached()
            .expect("encode");

        let (_a, mut alice) = temp_device(&AccountKey::generate().expect("account"));
        let chat = alice.create_chat().expect("chat");
        assert!(matches!(
            alice.invite(&chat, &key_package),
            Err(CoreError::Malformed { .. })
        ));
        assert_eq!(alice.outbox().expect("outbox"), Vec::<Outgoing>::new());
        assert_eq!(alice.chats[&chat].members().count(), 1);
    }

    #[test]
    fn every_chat_publishes_this_devices_mailbox() {
        let (_a, mut alice) = temp_device(&AccountKey::generate().expect("account"));
        let chat = alice.create_chat().expect("chat");
        let leaf = alice.chats[&chat].own_leaf_node().expect("own leaf");
        assert_eq!(mailbox_of(leaf).expect("mailbox"), alice.mailbox());
        assert_eq!(alice.chats(), vec![chat]);
    }

    #[test]
    fn chat_list_round_trips() {
        let chats: HashSet<ChatId> = [ChatId(vec![1; 16]), ChatId(vec![2; 16]), ChatId(vec![])]
            .into_iter()
            .collect();
        assert_eq!(
            decode_chat_list(&encode_chat_list(&chats)).expect("decode"),
            chats
        );
        assert!(decode_chat_list(&[5, 1, 2]).is_err());
    }

    #[test]
    fn creating_twice_in_one_store_is_refused() {
        let account = AccountKey::generate().expect("account");
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("device.db");
        let master = MasterKey::generate().expect("master key");
        drop(Device::create(&path, &master, &account).expect("first"));
        assert!(matches!(
            Device::create(&path, &master, &account),
            Err(CoreError::AlreadyInitialized)
        ));
        assert!(
            Device::open(&path, &master)
                .expect("open")
                .holds_account_key()
        );
    }
}
