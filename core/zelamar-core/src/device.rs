//! A device as a participant in MLS chats (ADR-0003).
//!
//! Every chat is an MLS group, a 1:1 chat included. A [`Device`] owns its MLS
//! signature key, a certificate from its account, and the state of every chat
//! it belongs to. It turns plaintext [`AppMessage`]s into opaque MLS bytes and
//! back, and refuses any member whose device certificate does not verify.

use std::collections::HashMap;

use openmls::prelude::{tls_codec::Deserialize as _, tls_codec::Serialize as _, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::{OpenMlsProvider, random::OpenMlsRand};

use crate::content::AppMessage;
use crate::error::{CoreError, Result};
use crate::identity::{AccountId, AccountKey, DeviceCertificate, DeviceId};

/// The one ciphersuite Zelamar speaks: hybrid post-quantum key exchange
/// (ML-KEM-768 + X25519), AES-256-GCM, SHA-384, Ed25519 signatures.
///
/// It comes from the IETF MLS post-quantum draft. Zelamar controls every
/// client, so if the draft changes, new chats move to the final codepoint.
pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519;

/// Messages are padded to a multiple of this many bytes so their length
/// leaks less about their content.
const PADDING: usize = 64;

/// Identifier of a chat: the MLS group id (16 random bytes).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChatId(Vec<u8>);

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Author {
    /// The account, as proven by its device certificate.
    pub account: AccountId,
    /// The device that sent the message.
    pub device: DeviceId,
}

/// Result of an invitation: the commit for existing members and the welcome
/// for the newcomer.
#[derive(Debug, Clone)]
pub struct Invitation {
    /// Goes to everyone already in the chat.
    pub commit: Vec<u8>,
    /// Goes to the invited device only.
    pub welcome: Vec<u8>,
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
}

/// One Zelamar device: its keys and the chats it takes part in.
pub struct Device {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    certificate: DeviceCertificate,
    chats: HashMap<ChatId, MlsGroup>,
}

impl Device {
    /// Creates a device for `account`: a fresh MLS signature key, certified
    /// by the account key.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] or [`CoreError::Mls`] if key generation or
    /// storage fails.
    pub fn new(account: &AccountKey) -> Result<Self> {
        let provider = OpenMlsRustCrypto::default();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|e| CoreError::crypto("generating the device signature key", e))?;
        signer
            .store(provider.storage())
            .map_err(|e| CoreError::mls("storing the device signature key", e))?;

        let device_id = DeviceId::random()?;
        let certificate = account.certify_device(device_id, signer.public())?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(certificate.encode()).into(),
            signature_key: signer.public().into(),
        };
        Ok(Self {
            provider,
            signer,
            credential,
            certificate,
            chats: HashMap::new(),
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

    /// Publishes a one-time `KeyPackage`: lets others add this device to a chat
    /// while it is offline. The private part stays in this device's storage.
    ///
    /// # Errors
    ///
    /// [`CoreError::Mls`] if the `KeyPackage` cannot be built or encoded.
    pub fn key_package(&self) -> Result<Vec<u8>> {
        let bundle = KeyPackage::builder()
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

    /// Starts a new chat with this device as its only member.
    ///
    /// # Errors
    ///
    /// [`CoreError::Crypto`] or [`CoreError::Mls`] if the group cannot be
    /// created.
    pub fn create_chat(&mut self) -> Result<ChatId> {
        let group_id: [u8; 16] = self
            .provider
            .rand()
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a chat id", e))?;
        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &create_config(),
            GroupId::from_slice(&group_id),
            self.credential.clone(),
        )
        .map_err(|e| CoreError::mls("creating a chat", e))?;
        let chat = ChatId::of(group.group_id());
        self.chats.insert(chat.clone(), group);
        Ok(chat)
    }

    /// Adds the device behind `key_package` to `chat`.
    ///
    /// The newcomer's device certificate is verified first; an unverifiable
    /// identity never enters a chat.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], [`CoreError::Unauthenticated`],
    /// [`CoreError::Malformed`] or [`CoreError::Mls`].
    pub fn invite(&mut self, chat: &ChatId, key_package: &[u8]) -> Result<Invitation> {
        let key_package = KeyPackageIn::tls_deserialize_exact(key_package)
            .map_err(|_| CoreError::malformed("key package", "invalid encoding"))?
            .validate(self.provider.crypto(), ProtocolVersion::Mls10)
            .map_err(|e| CoreError::mls("validating a key package", e))?;
        if key_package.ciphersuite() != CIPHERSUITE {
            return Err(CoreError::malformed(
                "key package",
                "unsupported ciphersuite",
            ));
        }
        let leaf = key_package.leaf_node();
        verify_member(leaf.credential(), leaf.signature_key().as_slice())?;

        let group = self.chats.get_mut(chat).ok_or(CoreError::UnknownChat)?;
        let (commit, welcome, _group_info) = group
            .add_members(&self.provider, &self.signer, &[key_package])
            .map_err(|e| CoreError::mls("adding a member", e))?;
        group
            .merge_pending_commit(&self.provider)
            .map_err(|e| CoreError::mls("applying an invitation", e))?;
        Ok(Invitation {
            commit: encode(&commit, "encoding a commit")?,
            welcome: encode(&welcome, "encoding a welcome")?,
        })
    }

    /// Joins a chat from a welcome addressed to this device.
    ///
    /// Every existing member's device certificate is verified before the
    /// chat is accepted.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`], [`CoreError::Unauthenticated`] or
    /// [`CoreError::Mls`].
    pub fn join(&mut self, welcome: &[u8]) -> Result<ChatId> {
        let message = MlsMessageIn::tls_deserialize_exact(welcome)
            .map_err(|_| CoreError::malformed("welcome", "invalid encoding"))?;
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            return Err(CoreError::malformed("welcome", "not a welcome message"));
        };
        let staged = StagedWelcome::new_from_welcome(&self.provider, &join_config(), welcome, None)
            .map_err(|e| CoreError::mls("reading a welcome", e))?;
        for member in staged.members() {
            verify_member(&member.credential, &member.signature_key)?;
        }
        let group = staged
            .into_group(&self.provider)
            .map_err(|e| CoreError::mls("joining a chat", e))?;
        let chat = ChatId::of(group.group_id());
        self.chats.insert(chat.clone(), group);
        Ok(chat)
    }

    /// Encrypts `message` for everyone in `chat`.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownChat`], [`CoreError::TooLarge`] or
    /// [`CoreError::Mls`].
    pub fn send(&mut self, chat: &ChatId, message: &AppMessage) -> Result<Vec<u8>> {
        let plaintext = message.encode()?;
        let group = self.chats.get_mut(chat).ok_or(CoreError::UnknownChat)?;
        let out = group
            .create_message(&self.provider, &self.signer, &plaintext)
            .map_err(|e| CoreError::mls("encrypting a message", e))?;
        encode(&out, "encoding a message")
    }

    /// Decrypts and authenticates an incoming MLS message.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`], [`CoreError::UnknownChat`],
    /// [`CoreError::Unauthenticated`] or [`CoreError::Mls`]. A message that
    /// fails here must be dropped, never shown.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Received> {
        let message = MlsMessageIn::tls_deserialize_exact(bytes)
            .map_err(|_| CoreError::malformed("MLS message", "invalid encoding"))?;
        let protocol = message
            .try_into_protocol_message()
            .map_err(|_| CoreError::malformed("MLS message", "not a chat message"))?;
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
                Ok(Received::Message {
                    chat,
                    author: Author {
                        account: certificate.account(),
                        device: certificate.device(),
                    },
                    message,
                })
            }
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                for added in staged.add_proposals() {
                    let leaf = added.add_proposal().key_package().leaf_node();
                    verify_member(leaf.credential(), leaf.signature_key().as_slice())?;
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

fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(PADDING)
        .build()
}

fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .padding_size(PADDING)
        .build()
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

    #[test]
    fn chats_use_the_post_quantum_ciphersuite() {
        let account = AccountKey::generate().expect("account");
        let mut device = Device::new(&account).expect("device");
        let chat = device.create_chat().expect("chat");
        assert_eq!(device.chats[&chat].ciphersuite(), CIPHERSUITE);
    }

    /// Key substitution: Mallory presents Alice's genuine certificate but
    /// her own signature key. The invite must be refused.
    #[test]
    fn a_borrowed_certificate_is_refused() {
        let alice_account = AccountKey::generate().expect("account");
        let mut alice = Device::new(&alice_account).expect("device");
        let mallory_account = AccountKey::generate().expect("account");
        let mut mallory = Device::new(&mallory_account).expect("device");

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
        let mut mallory = Device::new(&mallory_account).expect("device");

        // Mallory re-labels her certificate as Alice's account.
        let mut forged = mallory.certificate.encode();
        forged[1..33].copy_from_slice(alice_account.id().as_bytes());
        mallory.credential.credential = BasicCredential::new(forged).into();
        let key_package = mallory.key_package().expect("key package");

        let mut alice = Device::new(&alice_account).expect("device");
        let chat = alice.create_chat().expect("chat");
        assert!(matches!(
            alice.invite(&chat, &key_package),
            Err(CoreError::Unauthenticated(_))
        ));
    }
}
