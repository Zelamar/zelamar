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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use openmls::prelude::{tls_codec::Deserialize as _, tls_codec::Serialize as _, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::{OpenMlsProvider, random::OpenMlsRand, signatures::Signer};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::contacts::{self, Contact, Contacts};
use crate::content::{AppMessage, Body, MessageId};
use crate::envelope::{EnvelopeId, MailboxId};
use crate::error::{CoreError, Result};
use crate::history::{self, HistoryEntry, HistoryReader, SendStatus, StoredMessage};
use crate::identity::{AccountId, AccountKey, DeviceCertificate, DeviceId};
use crate::service::DirectoryEntry;
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
    /// Deduplication id, and the handle for [`Device::confirm_sent`].
    pub id: EnvelopeId,
    /// The other members' devices; never this one.
    pub recipients: Vec<MailboxId>,
    /// The MLS message.
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
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
    introduced: BTreeSet<ChatId>,
    persisted_introduced: BTreeSet<ChatId>,
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
        let store = Store::open(path, master)?;
        if store.record(RECORD_CERTIFICATE)?.is_some() {
            return Err(CoreError::AlreadyInitialized);
        }

        let provider = OpenMlsRustCrypto::default();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|e| CoreError::crypto("generating the device signature key", e))?;
        signer
            .store(provider.storage())
            .map_err(|e| CoreError::mls("storing the device signature key", e))?;
        let certificate = account.certify_device(DeviceId::random()?, signer.public())?;

        let account_key = AccountKey::from_secret_bytes(&account.to_secret_bytes()?)?;
        let mailbox = MailboxId::random()?;
        let mailbox_secret = Zeroizing::new(
            provider
                .rand()
                .random_array::<32>()
                .map_err(|e| CoreError::crypto("drawing a mailbox secret", e))?,
        );
        store.apply(&Batch {
            records: vec![
                (RECORD_CERTIFICATE, certificate.encode()),
                (RECORD_ACCOUNT_KEY, account.to_secret_bytes()?.to_vec()),
                (RECORD_MAILBOX, mailbox.0.to_vec()),
                (RECORD_MAILBOX_SECRET, mailbox_secret.to_vec()),
            ],
            ..Batch::default()
        })?;

        let mut device = Self {
            credential: credential_for(&certificate, &signer),
            provider,
            signer,
            certificate,
            account_key: Some(account_key),
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
        };
        device.persist()?;
        Ok(device)
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
        let contacts = store
            .record(RECORD_CONTACTS)?
            .map(|bytes| contacts::decode(&bytes))
            .transpose()?
            .unwrap_or_default();
        let introduced: BTreeSet<ChatId> =
            decode_chat_list(&store.record(RECORD_INTRODUCED)?.unwrap_or_default())?
                .into_iter()
                .collect();

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
        let agrees = self
            .contacts
            .get(&entry.account)
            .is_some_and(|contact| contact.username.as_deref() == Some(entry.username.as_str()));
        if agrees {
            self.transact(|device| {
                contacts::confirmed(&mut device.contacts, entry.account, &entry.username);
                Ok(())
            })?;
        }
        Ok(agrees)
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
        self.transact(|device| {
            let Some(bytes) = device.store.outbox_item(&id.0)? else {
                return Ok(());
            };
            let item = Queued::decode(&bytes)?;
            device.pending.confirmed.push(id);
            if let Some((chat, message)) = item.message {
                let reader = ChatHistory {
                    store: &device.store,
                    chat: &chat,
                    pending: &device.pending.messages,
                };
                if let Some(mut record) = reader.load(message)? {
                    record.status = SendStatus::Sent;
                    device.pending.messages.push((chat, record));
                }
            }
            Ok(())
        })
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
    /// it out, in one commit, and introduces this device there.
    ///
    /// Each device must be certified by the entry's account itself, so the
    /// directory cannot slip in someone else's device. Whether that account is
    /// really the person the user means is what safety codes confirm.
    ///
    /// # Errors
    ///
    /// [`CoreError::Unauthenticated`] for a device of another account,
    /// [`CoreError::Malformed`] for no or broken key packages,
    /// [`CoreError::Mls`] or a storage error.
    pub fn start_chat(&mut self, entry: &DirectoryEntry) -> Result<(ChatId, Invitation)> {
        let (account, key_packages) = (entry.account, &entry.key_packages);
        if key_packages.is_empty() {
            return Err(CoreError::malformed("contact", "has no reachable device"));
        }
        self.transact(|device| {
            let mut packages = Vec::with_capacity(key_packages.len());
            let mut newcomers = Vec::with_capacity(key_packages.len());
            for bytes in key_packages {
                let key_package = device.validate_key_package(bytes)?;
                let leaf = key_package.leaf_node();
                newcomers.push(verify_leaf(leaf)?);
                if certificate_of(leaf.credential())?.account() != account {
                    return Err(CoreError::Unauthenticated(
                        "a device does not belong to the contact",
                    ));
                }
                packages.push(key_package);
            }
            let chat = device.new_chat()?;
            let invitation = device.add(&chat, &packages, newcomers)?;
            contacts::confirmed(&mut device.contacts, account, &entry.username);
            device.introduce(&chat)?;
            Ok((chat, invitation))
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
        let (commit, welcome, _group_info) = group
            .add_members(&self.provider, &self.signer, packages)
            .map_err(|e| CoreError::mls("adding a member", e))?;
        group
            .merge_pending_commit(&self.provider)
            .map_err(|e| CoreError::mls("applying an invitation", e))?;
        newcomers.sort_unstable();
        newcomers.dedup();
        let invitation = Invitation {
            commit: Outgoing {
                id: EnvelopeId::random()?,
                recipients: existing,
                payload: encode(&commit, "encoding a commit")?,
            },
            welcome: Outgoing {
                id: EnvelopeId::random()?,
                recipients: newcomers,
                payload: encode(&welcome, "encoding a welcome")?,
            },
        };
        self.queue(&invitation.commit, None)?;
        self.queue(&invitation.welcome, None)?;
        Ok(invitation)
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
                if let Body::Profile { username } = &message.body
                    && author.account != self.account()
                {
                    contacts::claimed(&mut self.contacts, author.account, username);
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
                    verify_leaf(added.add_proposal().key_package().leaf_node())?;
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
                Ok(Received::ChatUpdated { chat })
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
        Ok(())
    }

    /// Queues `outgoing` in the outbox, as part of the operation in progress.
    /// With no recipients there is nothing to deliver, so nothing is queued.
    fn queue(&mut self, outgoing: &Outgoing, message: Option<(ChatId, MessageId)>) -> Result<()> {
        if outgoing.recipients.is_empty() {
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
        let mut batch = Batch::default();
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
        .build())
}

fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(PADDING)
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
