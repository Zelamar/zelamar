//! The app's session: one device of one account, talking to the delivery
//! service through Dart.
//!
//! Dart moves bytes (HTTPS and WebSocket); everything that signs, encrypts,
//! verifies or decides lives in the core (ADR-0012). Each method either
//! builds a request ([`HttpCall`]) or reads the answer to one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use zelamar_core::attachment::{self, BlobId, SealedFile};
use zelamar_core::contacts::Contact;
use zelamar_core::content::{AppMessage, Body, CallEndReason, FileRef, MessageId, ReceiptStatus};
use zelamar_core::device::{Author, ChatId, ChatSummary, Device, Received, Settled};
use zelamar_core::history::{CallRecord, HistoryEntry, SendStatus, StoredContent, StoredMessage};
use zelamar_core::identity::{AccountKey, DeviceId, SafetyCode};
use zelamar_core::lan;
use zelamar_core::link::{self, Applicant, Joining, Offer};
use zelamar_core::recovery::{KitUpload, Restoring};
use zelamar_core::service::{self, Request, CLAIM_WORK_BITS};
use zelamar_core::store::MasterKey;
use zelamar_core::CoreError;

/// File name of the encrypted store inside the session directory.
const STORE_FILE: &str = "zelamar.db";

/// A failure, with a stable code the app turns into words.
#[derive(Debug, Clone)]
pub struct SessionError {
    /// Stable code: the service's own (`username_taken`, `invalid_invite`,
    /// `unknown_account`, ...) or the core's (`unauthenticated`,
    /// `malformed`, `storage`, ...).
    pub code: String,
    /// Details for logs; never shown to people as is.
    pub message: String,
    /// Whether trying again later may work.
    pub transient: bool,
}

impl From<CoreError> for SessionError {
    fn from(error: CoreError) -> Self {
        let code = match &error {
            CoreError::Service { code, .. } => code.clone(),
            CoreError::Unauthenticated(_) => "unauthenticated".to_owned(),
            CoreError::Malformed { .. } => "malformed".to_owned(),
            CoreError::UnknownChat => "unknown_chat".to_owned(),
            CoreError::WrongKey => "wrong_key".to_owned(),
            CoreError::NotInitialized => "not_initialized".to_owned(),
            CoreError::AlreadyInitialized => "already_initialized".to_owned(),
            CoreError::Storage { .. } => "storage".to_owned(),
            CoreError::TooLarge { .. } => "too_large".to_owned(),
            _ => "internal".to_owned(),
        };
        Self {
            code,
            message: error.to_string(),
            transient: error.is_transient(),
        }
    }
}

fn failure(code: &str, message: &str) -> SessionError {
    SessionError {
        code: code.to_owned(),
        message: message.to_owned(),
        transient: false,
    }
}

/// One HTTP header.
#[derive(Debug, Clone)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// An HTTP request for the delivery service, ready to send as is.
#[derive(Debug, Clone)]
pub struct HttpCall {
    pub method: String,
    /// Relative to the service's base URL, starting with `/v0/`.
    pub path: String,
    pub headers: Vec<HttpHeader>,
    pub body: Vec<u8>,
}

impl From<Request> for HttpCall {
    fn from(request: Request) -> Self {
        Self {
            method: request.method.to_owned(),
            path: request.path,
            headers: request
                .headers
                .into_iter()
                .map(|(name, value)| HttpHeader {
                    name: name.to_owned(),
                    value,
                })
                .collect(),
            body: request.body,
        }
    }
}

/// How to read the device's mailbox: a WebSocket to `path` with this
/// `Authorization` header.
#[derive(Clone)]
pub struct MailboxLink {
    pub path: String,
    pub authorization: String,
}

/// One item waiting in the outbox, and the request that delivers it.
#[derive(Debug, Clone)]
pub struct PendingSend {
    /// Pass back to [`Session::sent`] with the answer.
    pub id: Vec<u8>,
    pub call: HttpCall,
}

/// What one envelope from the mailbox did.
#[derive(Debug, Clone)]
pub struct InboxEvent {
    /// Send this frame back over the WebSocket; `None` leaves the envelope
    /// for a later attempt.
    pub ack: Option<Vec<u8>>,
    /// The chat that changed, if any.
    pub chat: Option<String>,
    /// Someone started a chat with this device.
    pub joined: bool,
    /// A change took this device out of the chat; it may have been
    /// removed from its account.
    pub left: bool,
    /// Why the envelope was refused, if it was.
    pub refused: Option<String>,
    /// A new message from someone else, worth a notification.
    pub notice: Option<Notice>,
    /// What a call message asks of this device (ADR-0019).
    pub call: Option<CallSignal>,
}

/// What a notification about a new message says.
#[derive(Debug, Clone)]
pub struct Notice {
    /// Who wrote: `@name`, or "Someone new" before their name is known.
    pub title: String,
    /// The text, shortened to fit a notification.
    pub body: String,
}

/// Longest notification text, in characters.
const NOTICE_CHARS: usize = 180;

/// How old an offer may be and still ring: the caller gives up after 45
/// seconds (ADR-0019).
const RING_WINDOW_MS: u64 = 60_000;

/// A call nobody ended that is older than this is over, as far as the
/// chat shows: its caller went away mid-call.
const CALL_FORGOTTEN_MS: u64 = 6 * 60 * 60 * 1000;

/// What a call message asks of this device (ADR-0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSignalKind {
    /// Someone calls: ring, if [`CallSignal::ring`] says so.
    Offer,
    /// A device picked up: the caller connects to it; the called person's
    /// other devices stop ringing.
    Answer,
    /// The call is over.
    End,
    /// The call reached the person called: their device is ringing.
    Ringing,
}

/// Why a call ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallEnding {
    /// Someone hung up; before an answer, the caller gave up.
    Hangup,
    /// The person called said no.
    Declined,
    /// Nobody answered in time.
    Missed,
    /// The person called is in another call.
    Busy,
    /// The connection could not be made, or was lost.
    Failed,
    /// A reason from a newer Zelamar.
    Other,
}

/// A call message for this device.
#[derive(Debug, Clone)]
pub struct CallSignal {
    pub kind: CallSignalKind,
    pub chat: String,
    /// The call's id, base64url.
    pub call: String,
    /// Sent by another of this account's devices.
    pub own: bool,
    /// The other person in the chat: `@bob`.
    pub peer: String,
    /// Whether the call starts with video ([`CallSignalKind::Offer`]).
    pub video: bool,
    /// The offer or the answer.
    pub sdp: Option<String>,
    /// For an offer: whether this device should ring now. Never for a
    /// group, a chat request, one's own call, a call already over, or an
    /// old one.
    pub ring: bool,
    /// For [`CallSignalKind::End`].
    pub ending: Option<CallEnding>,
    /// For [`CallSignalKind::End`]: how long the people talked.
    pub duration_ms: Option<u32>,
}

/// One envelope that may go to a neighbour on the local network, tagged
/// under that neighbour's nonce (ADR-0020).
#[derive(Debug, Clone)]
pub struct LanParcel {
    /// Which mailbox it is for, as the neighbour can recognize it.
    pub tag: Vec<u8>,
    /// The envelope's id: what the neighbour answers once it has it.
    pub id: Vec<u8>,
    /// The envelope itself.
    pub envelope: Vec<u8>,
}

/// One server that helps calls connect.
#[derive(Clone)]
pub struct IceServerView {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Where an outgoing message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    /// Waiting to go out (offline, or not yet acknowledged by the service).
    Queued,
    /// The service has it.
    Sent,
    /// On someone else's device.
    Delivered,
    /// Read by someone else.
    Read,
}

/// The message a reply answers, as a short quote.
#[derive(Debug, Clone)]
pub struct Quote {
    /// Its id, base64url.
    pub id: String,
    /// Its text, shortened; or a placeholder when it is deleted or missing.
    pub text: String,
    /// Whether the user wrote it.
    pub outgoing: bool,
}

/// One reaction and how many people chose it.
#[derive(Debug, Clone)]
pub struct Reaction {
    pub emoji: String,
    pub count: u32,
    /// Whether one of them is the user's own.
    pub mine: bool,
}

/// One message as the chat screen shows it.
#[derive(Debug, Clone)]
pub struct MessageView {
    /// Message id, base64url.
    pub id: String,
    /// Position in this device's history; pass the smallest one to load
    /// older messages.
    pub seq: i64,
    pub outgoing: bool,
    /// The text, or a placeholder for what cannot be shown.
    pub text: String,
    /// Deleted by its author; `text` is empty.
    pub deleted: bool,
    /// A kind this version cannot show; the app asks to update.
    pub unsupported: bool,
    pub edited: bool,
    /// Author's clock, UTC milliseconds: what the app displays.
    pub sent_at_ms: i64,
    /// Meaningful for outgoing messages only.
    pub status: MessageStatus,
    /// The message this one answers.
    pub reply_to: Option<Quote>,
    /// Reactions, most chosen first.
    pub reactions: Vec<Reaction>,
    /// The user's own text, still there: it can be edited and deleted.
    pub editable: bool,
    /// Who wrote it: `@bob`, or a short fingerprint for an unknown account.
    pub author: String,
    /// A photo, a document or a voice message.
    pub file: Option<FileView>,
    /// Something that happened to the chat, shown as a line rather than a
    /// bubble; `text` is empty then.
    pub event: Option<ChatEvent>,
    /// A call; `text` is empty then.
    pub call: Option<CallView>,
}

/// A file in a message, as the chat screen shows it (ADR-0016).
#[derive(Debug, Clone)]
pub struct FileView {
    pub name: String,
    /// Media type, such as `image/jpeg`.
    pub mime: String,
    /// Bytes.
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// A small JPEG to show until the file itself arrives.
    pub thumb: Option<Vec<u8>>,
    pub duration_ms: Option<u32>,
}

/// A file sealed and ready to upload: send every call, then
/// [`Session::send_file`].
#[derive(Debug, Clone)]
pub struct PreparedFile {
    /// Pass back to [`Session::send_file`].
    pub blob: String,
    /// One upload per chunk; read each answer with [`chunk_uploaded`].
    pub calls: Vec<HttpCall>,
}

/// How a call went (ADR-0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    /// Ringing, or going on.
    Ongoing,
    /// Someone picked up; `duration_ms` says for how long, once it is over.
    Talked,
    /// Nobody picked up in time.
    Missed,
    /// The person called said no.
    Declined,
    /// The caller gave up before anyone picked up.
    Cancelled,
    /// The person called was in another call.
    Busy,
    /// The connection could not be made.
    Failed,
}

/// A call in a chat, shown as a line.
#[derive(Debug, Clone)]
pub struct CallView {
    pub video: bool,
    pub outcome: CallOutcome,
    /// How long the people talked.
    pub duration_ms: Option<u32>,
}

/// What kind of thing happened to a chat (ADR-0015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatEventKind {
    /// The author named the group.
    Named,
    /// The author left.
    Left,
}

/// Something that happened to a chat.
#[derive(Debug, Clone)]
pub struct ChatEvent {
    pub kind: ChatEventKind,
    /// The new name, for [`ChatEventKind::Named`].
    pub name: Option<String>,
}

/// One person in a chat, for its details screen.
#[derive(Debug, Clone)]
pub struct MemberView {
    /// `@bob`, or a short fingerprint for an unknown account.
    pub title: String,
    /// Whether the directory confirmed the name.
    pub verified: bool,
}

/// Someone to add to a group: the name asked for and the directory's
/// answer to [`lookup_call`].
#[derive(Debug, Clone)]
pub struct Invitee {
    pub username: String,
    pub answer: HttpAnswer,
}

/// Longest quote in a reply, in characters.
const QUOTE_CHARS: usize = 120;

/// One chat as the chat list shows it.
#[derive(Debug, Clone)]
pub struct ChatPreview {
    /// Chat id, base64url.
    pub id: String,
    /// `@bob`, `@alice, @bob`, or a short fingerprint for an unknown account.
    pub title: String,
    /// Whether the directory confirmed every name in the title.
    pub verified: bool,
    pub last: Option<MessageView>,
    /// Messages from others not yet read, up to 99.
    pub unread: u32,
    /// Someone else started it and the user has not answered: show it as a
    /// request to accept or decline.
    pub request: bool,
    /// A group: named, or with more than one other person.
    pub group: bool,
}

/// The service's answer to one [`HttpCall`].
#[derive(Debug, Clone)]
pub struct HttpAnswer {
    pub status: u16,
    pub body: Vec<u8>,
}

/// A device asking to join this account (ADR-0014).
#[derive(Debug, Clone)]
pub struct LinkApplicant {
    /// What the device calls itself ("Android phone").
    pub label: String,
    /// Six digits, `482 913`; the new device shows the same.
    pub match_code: String,
}

/// One chat that lacks some of this account's devices.
#[derive(Debug, Clone)]
pub struct DeviceGap {
    /// Chat id, base64url.
    pub chat: String,
    /// Device ids, base64url: look each up with
    /// [`Session::device_lookup_call`].
    pub devices: Vec<String>,
}

/// What healing found: every registered device of the account, the chats
/// that lack some of them, and the chats that still hold removed ones.
#[derive(Debug, Clone)]
pub struct HealPlan {
    /// Device ids, base64url.
    pub devices: Vec<String>,
    /// Add these with [`Session::add_own_devices`].
    pub gaps: Vec<DeviceGap>,
    /// Take these out with [`Session::remove_own_devices`].
    pub stale: Vec<DeviceGap>,
    /// `false` once this device was removed from the account.
    pub this_device_listed: bool,
}

/// The answer to one [`devices_of_call`], with the names it asked about.
#[derive(Debug, Clone)]
pub struct DirectoryAnswer {
    pub usernames: Vec<String>,
    pub answer: HttpAnswer,
}

/// Devices another account's chat lacks after it recovered (ADR-0018):
/// add them with [`Session::add_member_devices`] when `first`, or once the
/// gap has lasted.
#[derive(Debug, Clone)]
pub struct MemberGap {
    /// Chat id, base64url.
    pub chat: String,
    /// The account's username, normalized.
    pub username: String,
    /// Device ids, base64url.
    pub devices: Vec<String>,
    /// Whether this device is the one to add them now.
    pub first: bool,
}

/// One device of this account, for the devices screen.
#[derive(Debug, Clone)]
pub struct DeviceView {
    /// Device id, base64url.
    pub id: String,
    /// What it calls itself, if this device knows.
    pub label: Option<String>,
    pub this_device: bool,
}

/// One device of one account, stored encrypted in a directory.
pub struct Session {
    device: Device,
    /// The link this device is offering, while it shows a code.
    offer: Option<Offer>,
    /// The device that answered the offer, once one did.
    applicant: Option<Applicant>,
    /// Files sealed for upload and not sent yet, by blob id.
    prepared: HashMap<[u8; 16], SealedFile>,
    /// The recovery kit on its way to the service.
    kit: Option<KitUpload>,
}

/// A new device joining an existing account by its link code, before it
/// has a session of its own (ADR-0014).
#[flutter_rust_bridge::frb(opaque)]
pub struct Joiner {
    joining: Option<Joining>,
}

impl Joiner {
    /// Prepares to join the account whose device shows `code`; this
    /// device calls itself `label`.
    #[flutter_rust_bridge::frb(sync)]
    pub fn new(code: String, label: String) -> Result<Self, SessionError> {
        Ok(Self {
            joining: Some(Joining::new(&code, &label)?),
        })
    }

    fn joining(&self) -> Result<&Joining, SessionError> {
        self.joining
            .as_ref()
            .ok_or_else(|| failure("already_linked", "this link is finished"))
    }

    /// Six digits the other device must show too.
    #[flutter_rust_bridge::frb(sync)]
    pub fn match_code(&self) -> Result<String, SessionError> {
        Ok(self.joining()?.match_code())
    }

    /// Leaves this device's sealed request for the other device.
    #[flutter_rust_bridge::frb(sync)]
    pub fn request_call(&self) -> Result<HttpCall, SessionError> {
        Ok(self.joining()?.request()?.into())
    }

    /// Reads the answer to [`Joiner::request_call`]. `link_taken` means
    /// another device used this code first.
    #[flutter_rust_bridge::frb(sync)]
    pub fn requested(&self, status: u16, body: Vec<u8>) -> Result<(), SessionError> {
        Ok(link::stored(status, &body)?)
    }

    /// Asks whether the other device approved.
    #[flutter_rust_bridge::frb(sync)]
    pub fn check_call(&self) -> Result<HttpCall, SessionError> {
        Ok(self.joining()?.check().into())
    }

    /// Clears the meeting place on the service; send it once done.
    #[flutter_rust_bridge::frb(sync)]
    pub fn close_call(&self) -> Result<HttpCall, SessionError> {
        Ok(self.joining()?.close().into())
    }

    /// Reads the answer to [`Joiner::check_call`]. Once the other device
    /// approved, creates this device's session in `directory`, under
    /// `master_key`; then register it with [`Session::register`].
    pub fn finish(
        &mut self,
        status: u16,
        body: Vec<u8>,
        directory: String,
        master_key: Vec<u8>,
    ) -> Result<Option<Session>, SessionError> {
        let Some(linked) = self.joining()?.linked(status, &body)? else {
            return Ok(None);
        };
        let joining = self
            .joining
            .take()
            .ok_or_else(|| failure("already_linked", "this link is finished"))?;
        std::fs::create_dir_all(&directory)
            .map_err(|e| failure("storage", &format!("creating the directory: {e}")))?;
        let device = Device::create_linked(
            &store_path(&directory),
            &master(&master_key)?,
            joining,
            linked,
        )?;
        Ok(Some(Session::with(device)))
    }
}

/// A new device recovering an account with its recovery key (ADR-0018),
/// before it has a session of its own.
#[flutter_rust_bridge::frb(opaque)]
pub struct Restorer {
    restoring: Option<Restoring>,
}

impl Restorer {
    /// Prepares to recover `username` with the recovery key typed as `key`.
    /// `malformed` for a bad name or a mistyped key.
    #[flutter_rust_bridge::frb(sync)]
    pub fn new(username: String, key: String) -> Result<Self, SessionError> {
        Ok(Self {
            restoring: Some(Restoring::new(&username, &key)?),
        })
    }

    fn restoring(&self) -> Result<&Restoring, SessionError> {
        self.restoring
            .as_ref()
            .ok_or_else(|| failure("already_initialized", "this recovery is finished"))
    }

    /// Asks the service for the account's sealed kit.
    #[flutter_rust_bridge::frb(sync)]
    pub fn fetch_call(&self) -> Result<HttpCall, SessionError> {
        Ok(self.restoring()?.fetch()?.into())
    }

    /// Reads the answer to [`Restorer::fetch_call`], opens the kit and
    /// creates this device's session in `directory`, under `master_key`;
    /// then register it with [`Session::register`]. `wrong_recovery_key`
    /// when the name or the key is wrong.
    pub fn finish(
        &mut self,
        status: u16,
        body: Vec<u8>,
        directory: String,
        master_key: Vec<u8>,
    ) -> Result<Session, SessionError> {
        let restoring = self
            .restoring
            .take()
            .ok_or_else(|| failure("already_initialized", "this recovery is finished"))?;
        let recovered = restoring
            .recovered(status, &body)
            .map_err(|error| match error {
                CoreError::Unauthenticated(_) => {
                    failure("wrong_recovery_key", "the kit did not open")
                }
                CoreError::Service { code, .. } if code == "wrong_key" => {
                    failure("wrong_recovery_key", "no kit for this name and key")
                }
                other => other.into(),
            })?;
        std::fs::create_dir_all(&directory)
            .map_err(|e| failure("storage", &format!("creating the directory: {e}")))?;
        let device =
            Device::create_recovered(&store_path(&directory), &master(&master_key)?, recovered)?;
        Ok(Session::with(device))
    }
}

/// Takes one key package of one device of `username`, to add it to a chat
/// with [`Session::add_member_devices`] (ADR-0018).
#[flutter_rust_bridge::frb(sync)]
pub fn member_device_call(username: String, device: String) -> Result<HttpCall, SessionError> {
    Ok(service::device_lookup(&username, device_id(&device)?)?.into())
}

/// Asks the directory for the devices of up to 64 `usernames` at once
/// (ADR-0018).
#[flutter_rust_bridge::frb(sync)]
pub fn devices_of_call(usernames: Vec<String>) -> Result<HttpCall, SessionError> {
    Ok(service::devices_of(&usernames)?.into())
}

/// A fresh random master key for a new session. Keep it in the platform's
/// secure storage; it opens the session's store.
#[flutter_rust_bridge::frb(sync)]
pub fn new_master_key() -> Result<Vec<u8>, SessionError> {
    Ok(MasterKey::generate()?.expose().to_vec())
}

/// Whether `directory` already holds a session.
#[flutter_rust_bridge::frb(sync)]
#[must_use]
pub fn session_exists(directory: String) -> bool {
    store_path(&directory).exists()
}

/// Reads the answer to [`Session::push_call`].
#[flutter_rust_bridge::frb(sync)]
pub fn push_registered(status: u16, body: Vec<u8>) -> Result<(), SessionError> {
    Ok(service::push_set(status, &body)?)
}

/// Reads the answer to one of [`PreparedFile::calls`].
#[flutter_rust_bridge::frb(sync)]
pub fn chunk_uploaded(status: u16, body: Vec<u8>) -> Result<(), SessionError> {
    Ok(attachment::uploaded(status, &body)?)
}

/// Deletes the store in `directory`: for an account that never got a name,
/// before this device joins another account instead. Dispose its session
/// first.
#[flutter_rust_bridge::frb(sync)]
pub fn remove_store(directory: String) -> Result<(), SessionError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let path = PathBuf::from(&directory).join(format!("{STORE_FILE}{suffix}"));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(failure("storage", &format!("removing the store: {error}")));
            }
        }
    }
    Ok(())
}

/// Looks up `username` with one key package per device, to start a chat.
#[flutter_rust_bridge::frb(sync)]
pub fn lookup_call(username: String) -> Result<HttpCall, SessionError> {
    Ok(service::lookup(&username)?.into())
}

/// Asks only which account holds `username`, to confirm a claimed name.
#[flutter_rust_bridge::frb(sync)]
pub fn key_lookup_call(username: String) -> Result<HttpCall, SessionError> {
    Ok(service::key_lookup(&username)?.into())
}

/// Lowercased `username` if it is well-formed, else `None`.
#[flutter_rust_bridge::frb(sync)]
#[must_use]
pub fn normalize_username(username: String) -> Option<String> {
    service::normalize_username(&username).ok()
}

impl Session {
    /// Creates a new account and its first device in `directory`.
    pub fn create(directory: String, master_key: Vec<u8>) -> Result<Self, SessionError> {
        std::fs::create_dir_all(&directory)
            .map_err(|e| failure("storage", &format!("creating the directory: {e}")))?;
        let account = AccountKey::generate()?;
        let device = Device::create(&store_path(&directory), &master(&master_key)?, &account)?;
        Ok(Self::with(device))
    }

    /// Opens the session stored in `directory`.
    pub fn open(directory: String, master_key: Vec<u8>) -> Result<Self, SessionError> {
        let device = Device::open(&store_path(&directory), &master(&master_key)?)?;
        Ok(Self::with(device))
    }

    fn with(device: Device) -> Self {
        Self {
            device,
            offer: None,
            applicant: None,
            prepared: HashMap::new(),
            kit: None,
        }
    }

    /// The recovery key, as shown; only the device holding the account key
    /// has one, once made.
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn recovery_key(&self) -> Option<String> {
        self.device.recovery_key()
    }

    /// Makes a new recovery key, replacing any earlier one; leave the kit at
    /// the service with [`Session::recovery_call`] next.
    pub fn new_recovery_key(&mut self) -> Result<String, SessionError> {
        Ok(self.device.new_recovery_key()?)
    }

    /// The recovery kit to leave at the service, or `None` when it already
    /// holds the current one (or there is no key).
    pub fn recovery_call(&mut self) -> Result<Option<HttpCall>, SessionError> {
        self.kit = self.device.recovery_upload(now_ms())?;
        Ok(self.kit.as_ref().map(|kit| kit.request.clone().into()))
    }

    /// Reads the answer to [`Session::recovery_call`].
    pub fn recovery_saved(&mut self, status: u16, body: Vec<u8>) -> Result<(), SessionError> {
        let Some(kit) = self.kit.take() else {
            return Ok(());
        };
        Ok(self.device.recovery_uploaded(&kit, status, &body)?)
    }

    /// The names whose devices healing looks up for other accounts, this
    /// account's own first: everyone known by name in the chats this
    /// account has spoken in (ADR-0018). Ask in batches of 64 with
    /// [`devices_of_call`].
    pub fn directory_names(&self) -> Result<Vec<String>, SessionError> {
        let accounts = self.device.chat_accounts()?;
        let mut names = vec![self.own_name()?];
        names.extend(
            self.device
                .contacts()
                .into_iter()
                .filter(|contact| accounts.contains(&contact.account))
                .filter_map(|contact| contact.username),
        );
        Ok(names)
    }

    /// Reads the answers to [`devices_of_call`] for the names from
    /// [`Session::directory_names`]: the chats that lack another account's
    /// devices since it recovered.
    pub fn member_gaps(
        &self,
        answers: Vec<DirectoryAnswer>,
    ) -> Result<Vec<MemberGap>, SessionError> {
        let mut names = HashMap::new();
        let mut lists = Vec::new();
        for answer in &answers {
            for (name, account, devices) in
                service::device_lists(&answer.usernames, answer.answer.status, &answer.answer.body)?
            {
                names.insert(account, name);
                lists.push((account, devices));
            }
        }
        Ok(self
            .device
            .foreign_gaps(&lists)?
            .into_iter()
            .filter_map(|gap| {
                Some(MemberGap {
                    chat: chat_key(&gap.chat),
                    username: names.get(&gap.account)?.clone(),
                    devices: gap.devices.iter().map(device_key).collect(),
                    first: gap.first,
                })
            })
            .collect())
    }

    /// Adds devices of `username`, whose account is in `chat`, from the
    /// answers to [`member_device_call`] for each, in one commit.
    pub fn add_member_devices(
        &mut self,
        chat: String,
        username: String,
        answers: Vec<HttpAnswer>,
    ) -> Result<(), SessionError> {
        let entries = answers
            .iter()
            .map(|answer| service::directory_entry(&username, answer.status, &answer.body))
            .collect::<Result<Vec<_>, _>>()?;
        let account = entries
            .first()
            .map(|entry| entry.account)
            .ok_or_else(|| failure("malformed", "no device to add"))?;
        let chat = self.find_chat(&chat)?;
        self.device.add_devices(&chat, account, &entries)?;
        Ok(())
    }

    /// Whether this device can link others: only the one holding the
    /// account key can.
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn can_link(&self) -> bool {
        self.device.holds_account_key()
    }

    /// Starts linking a new device: returns the code to show,
    /// `xxxxx-xxxxx-xxxxx-xxxxx`. A new offer replaces any earlier one.
    pub fn offer_link(&mut self) -> Result<String, SessionError> {
        if !self.device.holds_account_key() {
            return Err(failure(
                "unauthenticated",
                "only the device that created the account can link devices",
            ));
        }
        let offer = Offer::new()?;
        let code = offer.code();
        self.offer = Some(offer);
        self.applicant = None;
        Ok(code)
    }

    fn offer(&self) -> Result<&Offer, SessionError> {
        self.offer
            .as_ref()
            .ok_or_else(|| failure("no_link", "no link is being offered"))
    }

    /// Asks whether a new device answered the code.
    #[flutter_rust_bridge::frb(sync)]
    pub fn link_check_call(&self) -> Result<HttpCall, SessionError> {
        Ok(self.offer()?.check().into())
    }

    /// Reads the answer to [`Session::link_check_call`]: the device that
    /// asked to join, once one did.
    pub fn link_applicant(
        &mut self,
        status: u16,
        body: Vec<u8>,
    ) -> Result<Option<LinkApplicant>, SessionError> {
        let applicant = self.offer()?.applicant(status, &body)?;
        self.applicant.clone_from(&applicant);
        Ok(applicant.map(|applicant| LinkApplicant {
            label: applicant.label,
            match_code: applicant.match_code,
        }))
    }

    /// Approves the device that asked: its certificate, this account's
    /// name and contacts go to it, sealed.
    pub fn approve_link(&mut self) -> Result<HttpCall, SessionError> {
        let applicant = self
            .applicant
            .as_ref()
            .ok_or_else(|| failure("no_link", "no device asked to join"))?;
        let offer = self
            .offer
            .as_ref()
            .ok_or_else(|| failure("no_link", "no link is being offered"))?;
        Ok(self.device.approve_link(offer, applicant)?.into())
    }

    /// Reads the answer to [`Session::approve_link`]; returns the new
    /// device's id, base64url. Then heal the chats once it registered.
    pub fn link_approved(&mut self, status: u16, body: Vec<u8>) -> Result<String, SessionError> {
        link::stored(status, &body)?;
        let applicant = self
            .applicant
            .take()
            .ok_or_else(|| failure("no_link", "no device asked to join"))?;
        self.offer = None;
        Ok(URL_SAFE_NO_PAD.encode(applicant.device.as_bytes()))
    }

    /// Stops offering a link.
    #[flutter_rust_bridge::frb(sync)]
    pub fn cancel_link(&mut self) {
        self.offer = None;
        self.applicant = None;
    }

    fn own_name(&self) -> Result<String, SessionError> {
        self.device
            .username()
            .map(str::to_owned)
            .ok_or_else(|| failure("no_username", "claim a username first"))
    }

    /// Lists this account's devices, to heal chats (ADR-0014).
    #[flutter_rust_bridge::frb(sync)]
    pub fn devices_call(&self) -> Result<HttpCall, SessionError> {
        Ok(service::devices(&self.own_name()?)?.into())
    }

    /// Reads the answer to [`Session::devices_call`]: which chats lack
    /// which of this account's devices, and which still hold removed ones.
    pub fn heal_plan(&self, status: u16, body: Vec<u8>) -> Result<HealPlan, SessionError> {
        let (_, registered) = service::device_list(&self.own_name()?, status, &body)?;
        let gaps = |found: Vec<(ChatId, Vec<DeviceId>)>| {
            found
                .into_iter()
                .map(|(chat, devices)| DeviceGap {
                    chat: chat_key(&chat),
                    devices: devices.iter().map(device_key).collect(),
                })
                .collect()
        };
        Ok(HealPlan {
            devices: registered
                .iter()
                .map(|certificate| device_key(&certificate.device()))
                .collect(),
            gaps: gaps(self.device.missing_devices(&registered)?),
            stale: gaps(self.device.stale_devices(&registered)?),
            this_device_listed: registered
                .iter()
                .any(|certificate| certificate.device() == self.device.id()),
        })
    }

    /// Reads the answer to [`Session::devices_call`] for the devices
    /// screen: this device first.
    pub fn device_views(
        &self,
        status: u16,
        body: Vec<u8>,
    ) -> Result<Vec<DeviceView>, SessionError> {
        let (_, registered) = service::device_list(&self.own_name()?, status, &body)?;
        let mut views: Vec<DeviceView> = registered
            .iter()
            .map(|certificate| {
                let device = certificate.device();
                DeviceView {
                    id: device_key(&device),
                    label: self.device.device_label(device).map(str::to_owned),
                    this_device: device == self.device.id(),
                }
            })
            .collect();
        views.sort_by_key(|view| !view.this_device);
        Ok(views)
    }

    /// Remembers what this device calls itself; linking passes it on.
    pub fn set_own_label(&mut self, label: String) -> Result<(), SessionError> {
        let me = self.device.id();
        Ok(self.device.remember_label(me, &label)?)
    }

    /// Takes `device` out of the account: this device itself, or, on the
    /// device holding the account key, any other. Heal afterwards.
    pub fn remove_device_call(&self, device: String) -> Result<HttpCall, SessionError> {
        Ok(self
            .device
            .remove_device(&self.own_name()?, device_id(&device)?, now_ms())?
            .into())
    }

    /// Reads the answer to [`Session::remove_device_call`].
    #[flutter_rust_bridge::frb(sync)]
    pub fn device_removed(&self, status: u16, body: Vec<u8>) -> Result<(), SessionError> {
        Ok(service::device_removed(status, &body)?)
    }

    /// Takes removed devices of this account out of `chat` in one change.
    /// It goes out with the next [`Session::pending_sends`].
    pub fn remove_own_devices(
        &mut self,
        chat: String,
        devices: Vec<String>,
    ) -> Result<(), SessionError> {
        let devices = devices
            .iter()
            .map(|device| device_id(device))
            .collect::<Result<Vec<_>, _>>()?;
        let chat = self.find_chat(&chat)?;
        self.device.remove_own_devices(&chat, &devices)?;
        Ok(())
    }

    /// Takes one key package of one of this account's devices.
    #[flutter_rust_bridge::frb(sync)]
    pub fn device_lookup_call(&self, device: String) -> Result<HttpCall, SessionError> {
        Ok(service::device_lookup(&self.own_name()?, device_id(&device)?)?.into())
    }

    /// Adds this account's devices to `chat` in one change, from the
    /// answers to [`Session::device_lookup_call`]. It goes out with the
    /// next [`Session::pending_sends`].
    pub fn add_own_devices(
        &mut self,
        chat: String,
        answers: Vec<HttpAnswer>,
    ) -> Result<(), SessionError> {
        let name = self.own_name()?;
        let entries = answers
            .iter()
            .map(|answer| service::directory_entry(&name, answer.status, &answer.body))
            .collect::<Result<Vec<_>, _>>()?;
        let chat = self.find_chat(&chat)?;
        self.device.add_own_devices(&chat, &entries)?;
        Ok(())
    }

    /// The account key's fingerprint, for "this is me" screens:
    /// `a1b2 c3d4 e5f6 0718 9a0b c1d2 e3f4 0516`.
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn fingerprint(&self) -> String {
        self.device.account().as_bytes()[..16]
            .chunks(2)
            .map(|pair| {
                pair.iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The 60-digit safety code with the one other person in `chat`, as
    /// twelve groups of five digits; `None` in a chat with more people or
    /// nobody else. Two people compare it to know no one sits between them.
    pub fn safety_code(&self, chat: String) -> Result<Option<Vec<String>>, SessionError> {
        let summary = self
            .device
            .chat_list()?
            .into_iter()
            .find(|summary| chat_key(&summary.chat) == chat)
            .ok_or_else(|| failure("unknown_chat", "no such chat on this device"))?;
        Ok(match summary.members.as_slice() {
            [other] => Some(
                SafetyCode::between(&self.device.account(), &other.account)
                    .groups()
                    .iter()
                    .map(|group| format!("{group:05}"))
                    .collect(),
            ),
            _ => None,
        })
    }

    /// The username this account holds, once claimed.
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn username(&self) -> Option<String> {
        self.device.username().map(str::to_owned)
    }

    /// Claims `username` with a beta `invite`. Includes a proof of work of
    /// about a second; run it off the UI thread.
    pub fn claim(&self, username: String, invite: String) -> Result<HttpCall, SessionError> {
        self.claim_with(&username, &invite, CLAIM_WORK_BITS)
    }

    fn claim_with(&self, username: &str, invite: &str, bits: u8) -> Result<HttpCall, SessionError> {
        Ok(self.device.claim(username, invite, now_ms(), bits)?.into())
    }

    /// Reads the answer to [`Session::claim`]; returns the username held.
    pub fn claimed(&mut self, status: u16, body: Vec<u8>) -> Result<String, SessionError> {
        Ok(self.device.claimed(status, &body)?)
    }

    /// Registers this device (mailbox and key packages) under the claimed
    /// username.
    pub fn register(&mut self) -> Result<HttpCall, SessionError> {
        let username = self
            .device
            .username()
            .ok_or_else(|| failure("no_username", "claim a username first"))?
            .to_owned();
        Ok(self.device.register(&username, now_ms())?.into())
    }

    /// Reads the answer to [`Session::register`]: key packages stored.
    pub fn registered(&self, status: u16, body: Vec<u8>) -> Result<u32, SessionError> {
        let count = service::key_package_count(status, &body)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Starts a chat from the answer to [`lookup_call`] for `username`, and
    /// `own`, the answer to a lookup of this account's own name, so its
    /// other devices are in from the start. Returns the chat id.
    pub fn start_chat(
        &mut self,
        username: String,
        status: u16,
        body: Vec<u8>,
        own: Option<HttpAnswer>,
    ) -> Result<String, SessionError> {
        let entry = service::directory_entry(&username, status, &body)?;
        let siblings = match own {
            Some(answer) => vec![service::directory_entry(
                &self.own_name()?,
                answer.status,
                &answer.body,
            )?],
            None => Vec::new(),
        };
        let (chat, _) = self.device.start_chat(&entry, &siblings)?;
        Ok(chat_key(&chat))
    }

    /// Writes `text` into `chat`. It goes out with the next
    /// [`Session::pending_sends`].
    pub fn send_text(&mut self, chat: String, text: String) -> Result<(), SessionError> {
        self.send_body(&chat, Body::Text { text })
    }

    /// Writes `text` into `chat` as an answer to message `to`.
    pub fn reply_text(
        &mut self,
        chat: String,
        to: String,
        text: String,
    ) -> Result<(), SessionError> {
        let to = message_id(&to)?;
        self.send_body(&chat, Body::Reply { to, text })
    }

    /// Replaces the text of one of the user's own messages, for everyone.
    pub fn edit_text(
        &mut self,
        chat: String,
        target: String,
        text: String,
    ) -> Result<(), SessionError> {
        let target = message_id(&target)?;
        self.send_body(&chat, Body::Edit { target, text })
    }

    /// Deletes one of the user's own messages, for everyone.
    pub fn delete_message(&mut self, chat: String, target: String) -> Result<(), SessionError> {
        let target = message_id(&target)?;
        self.send_body(&chat, Body::Delete { target })
    }

    /// Sets the user's reaction to a message, or with `None` removes it.
    pub fn react(
        &mut self,
        chat: String,
        target: String,
        reaction: Option<String>,
    ) -> Result<(), SessionError> {
        let target = message_id(&target)?;
        self.send_body(&chat, Body::React { target, reaction })
    }

    fn send_body(&mut self, chat: &str, body: Body) -> Result<(), SessionError> {
        let chat = self.find_chat(chat)?;
        let message = AppMessage::new(MessageId::random()?, now_ms(), body);
        self.device.send(&chat, &message)?;
        Ok(())
    }

    /// Calls the other person in `chat`; `sdp` is this device's offer.
    /// Returns the call's id (ADR-0019).
    pub fn start_call(
        &mut self,
        chat: String,
        video: bool,
        sdp: String,
    ) -> Result<String, SessionError> {
        let chat = self.find_chat(&chat)?;
        if self.device.others_in(&chat)? != 1 {
            return Err(failure("not_one_to_one", "a call is between two people"));
        }
        let id = MessageId::random()?;
        let offer = AppMessage::new(id, now_ms(), Body::Call { video, sdp });
        self.device.send(&chat, &offer)?;
        Ok(URL_SAFE_NO_PAD.encode(id.0))
    }

    /// Picks up `call` in `chat`; `sdp` is this device's answer.
    pub fn answer_call(
        &mut self,
        chat: String,
        call: String,
        sdp: String,
    ) -> Result<(), SessionError> {
        let call = message_id(&call)?;
        self.send_body(&chat, Body::CallAnswer { call, sdp })
    }

    /// Ends `call` in `chat`, or turns it down; `duration_ms` is how long
    /// the people talked.
    pub fn end_call(
        &mut self,
        chat: String,
        call: String,
        ending: CallEnding,
        duration_ms: Option<u32>,
    ) -> Result<(), SessionError> {
        let call = message_id(&call)?;
        let reason = match ending {
            CallEnding::Hangup => CallEndReason::Hangup,
            CallEnding::Declined => CallEndReason::Declined,
            CallEnding::Missed => CallEndReason::Missed,
            CallEnding::Busy => CallEndReason::Busy,
            CallEnding::Failed => CallEndReason::Failed,
            CallEnding::Other => return Err(failure("malformed", "say why the call ends")),
        };
        self.send_body(
            &chat,
            Body::CallEnd {
                call,
                reason,
                duration_ms,
            },
        )
    }

    /// The envelopes that may go to neighbours on the local network while
    /// the service is out of reach, tagged under a neighbour's `nonce`
    /// (ADR-0020).
    pub fn lan_offer(&self, nonce: Vec<u8>) -> Result<Vec<LanParcel>, SessionError> {
        let nonce = lan_nonce(nonce)?;
        self.device
            .parcels()?
            .into_iter()
            .map(|parcel| {
                Ok(LanParcel {
                    tag: lan::tag(&parcel.mailbox, &nonce)?.to_vec(),
                    id: parcel.id.0.to_vec(),
                    envelope: parcel.envelope,
                })
            })
            .collect()
    }

    /// This device's mailbox tag under `nonce`, the one it gave a
    /// neighbour that asks what it hosts.
    #[flutter_rust_bridge::frb(sync)]
    pub fn lan_tag(&self, nonce: Vec<u8>) -> Result<Vec<u8>, SessionError> {
        Ok(self.device.own_tag(&lan_nonce(nonce)?)?.to_vec())
    }

    /// Asks for the servers that help this device's calls connect; read
    /// the answer with [`ice_servers`].
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn ice_call(&self) -> HttpCall {
        self.device.ice_request().into()
    }

    /// Everything waiting to go out, oldest first.
    pub fn pending_sends(&self) -> Result<Vec<PendingSend>, SessionError> {
        self.device
            .outbox()?
            .iter()
            .map(|outgoing| {
                Ok(PendingSend {
                    id: outgoing.id.0.to_vec(),
                    call: service::delivery(outgoing)?.into(),
                })
            })
            .collect()
    }

    /// Reads the service's answer to a [`PendingSend`]; on success the item
    /// leaves the outbox and its message counts as sent. Returns `true` when
    /// it was a change to a chat that lost to someone else's: whatever it
    /// was for must be done again (healing does so by itself).
    pub fn sent(&mut self, id: Vec<u8>, status: u16, body: Vec<u8>) -> Result<bool, SessionError> {
        let id: [u8; 16] = id
            .try_into()
            .map_err(|_| failure("malformed", "an outbox id is 16 bytes"))?;
        // A delivery is confirmed; a commit is applied or dropped as the
        // sequencer decided. Either way the item leaves the outbox.
        let settled = self
            .device
            .settle(zelamar_core::envelope::EnvelopeId(id), status, &body)?;
        Ok(settled == Settled::Refused)
    }

    /// Leaves this device's push token with its mailbox, or takes it back
    /// with `None` (ADR-0017).
    #[flutter_rust_bridge::frb(sync)]
    pub fn push_call(&self, token: Option<String>) -> Result<HttpCall, SessionError> {
        Ok(self.device.push_token(token.as_deref())?.into())
    }

    /// How to read this device's mailbox.
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn mailbox(&self) -> MailboxLink {
        let access = self.device.mailbox_access();
        MailboxLink {
            path: access.path,
            authorization: access.authorization,
        }
    }

    /// Opens one binary WebSocket message from the mailbox.
    pub fn open_envelope(&mut self, bytes: Vec<u8>) -> Result<InboxEvent, SessionError> {
        let opened = self.device.open_envelope(&bytes)?;
        let ack = (!opened.keep()).then(|| service::acknowledgement(&[opened.id]));
        Ok(match opened.result {
            Ok(Received::Joined { chat, by }) => InboxEvent {
                ack,
                chat: Some(chat_key(&chat)),
                joined: true,
                call: None,
                left: false,
                refused: None,
                // One's own other device bringing this one in is no news.
                notice: (by.account != self.device.account()).then(|| Notice {
                    title: self.name_of(by.account),
                    body: "New chat request".to_owned(),
                }),
            },
            Ok(Received::Message {
                chat,
                author,
                message,
            }) => InboxEvent {
                ack,
                chat: Some(chat_key(&chat)),
                joined: false,
                call: self.call_signal(&chat, author, &message),
                left: false,
                refused: None,
                notice: (author.account != self.device.account())
                    .then(|| self.notice(&chat, author.account, &message.body))
                    .flatten(),
            },
            Ok(Received::Left { chat }) => InboxEvent {
                ack,
                chat: Some(chat_key(&chat)),
                joined: false,
                call: None,
                left: true,
                refused: None,
                notice: None,
            },
            Ok(Received::ChatUpdated { chat } | Received::OwnEcho { chat }) => InboxEvent {
                ack,
                chat: Some(chat_key(&chat)),
                joined: false,
                call: None,
                left: false,
                refused: None,
                notice: None,
            },
            Ok(_) => InboxEvent {
                ack,
                chat: None,
                joined: false,
                call: None,
                left: false,
                refused: None,
                notice: None,
            },
            Err(error) => InboxEvent {
                ack,
                chat: None,
                joined: false,
                call: None,
                left: false,
                refused: Some(SessionError::from(error).code),
                notice: None,
            },
        })
    }

    /// Usernames that someone in a chat claims but the directory has not
    /// confirmed yet. Look each up with [`key_lookup_call`].
    #[must_use]
    pub fn names_to_confirm(&self) -> Vec<String> {
        self.device
            .unconfirmed_contacts()
            .into_iter()
            .filter_map(|contact| contact.username)
            .collect()
    }

    /// Reads the answer to [`key_lookup_call`] for `username`; returns
    /// whether the directory confirmed the claim.
    pub fn confirm_name(
        &mut self,
        username: String,
        status: u16,
        body: Vec<u8>,
    ) -> Result<bool, SessionError> {
        match service::directory_entry(&username, status, &body) {
            Ok(entry) => Ok(self.device.confirm_contact(&entry)?),
            // No one holds the name: whoever claimed it made it up.
            Err(CoreError::Service { code, .. }) if code == "unknown_account" => {
                self.device.refute_name(&username)?;
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Marks `chat` as read; in a chat the user has written in, the authors
    /// get a read receipt (it goes out with the next
    /// [`Session::pending_sends`]). Returns how many messages it covered.
    pub fn mark_read(&mut self, chat: String) -> Result<u32, SessionError> {
        let chat = self.find_chat(&chat)?;
        let count = self.device.mark_read(&chat)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Accepts a chat request: the people there learn this account's
    /// username and, from now on, get receipts.
    pub fn accept_chat(&mut self, chat: String) -> Result<(), SessionError> {
        let chat = self.find_chat(&chat)?;
        Ok(self.device.accept_chat(&chat)?)
    }

    /// Removes a chat from this device, history and all; declines a request.
    /// Nobody is told.
    pub fn delete_chat(&mut self, chat: String) -> Result<(), SessionError> {
        let chat = self.find_chat(&chat)?;
        Ok(self.device.delete_chat(&chat)?)
    }

    /// Starts a group called `name` with the people in `invitees`, and
    /// this account's other devices from `own` (a lookup of its own name).
    /// Returns the chat id.
    pub fn create_group(
        &mut self,
        name: String,
        invitees: Vec<Invitee>,
        own: Option<HttpAnswer>,
    ) -> Result<String, SessionError> {
        let members = entries(&invitees)?;
        let siblings = match own {
            Some(answer) => vec![service::directory_entry(
                &self.own_name()?,
                answer.status,
                &answer.body,
            )?],
            None => Vec::new(),
        };
        let (chat, _) = self.device.create_group(&name, &members, &siblings)?;
        Ok(chat_key(&chat))
    }

    /// Adds the people in `invitees` to `chat`.
    pub fn add_members(
        &mut self,
        chat: String,
        invitees: Vec<Invitee>,
    ) -> Result<(), SessionError> {
        let members = entries(&invitees)?;
        let chat = self.find_chat(&chat)?;
        self.device.add_members(&chat, &members)?;
        Ok(())
    }

    /// Renames `chat` for everyone in it.
    pub fn rename_chat(&mut self, chat: String, name: String) -> Result<(), SessionError> {
        let chat = self.find_chat(&chat)?;
        Ok(self.device.rename_chat(&chat, &name)?)
    }

    /// Leaves `chat` and erases it here; the others are told.
    pub fn leave_chat(&mut self, chat: String) -> Result<(), SessionError> {
        let chat = self.find_chat(&chat)?;
        Ok(self.device.leave_chat(&chat)?)
    }

    /// Takes the devices of people who left out of the chats where this
    /// device is the one to do it; returns how many chats changed. They go
    /// out with the next [`Session::pending_sends`].
    pub fn remove_departed(&mut self) -> Result<u32, SessionError> {
        let due = self.device.departures()?;
        for chat in &due {
            self.device.remove_departed(chat)?;
        }
        Ok(u32::try_from(due.len()).unwrap_or(u32::MAX))
    }

    /// Everyone else in `chat`.
    pub fn members(&self, chat: String) -> Result<Vec<MemberView>, SessionError> {
        let summary = self
            .device
            .chat_list()?
            .into_iter()
            .find(|summary| chat_key(&summary.chat) == chat)
            .ok_or_else(|| failure("unknown_chat", "no such chat on this device"))?;
        Ok(summary
            .members
            .iter()
            .map(|member| MemberView {
                title: member_title(member),
                verified: member.verified,
            })
            .collect())
    }

    /// Seals `bytes` for upload under a fresh key (ADR-0016).
    pub fn prepare_file(&mut self, bytes: Vec<u8>) -> Result<PreparedFile, SessionError> {
        let sealed = attachment::seal_file(&bytes)?;
        let calls = (0..u32::try_from(sealed.chunks.len()).unwrap_or(0))
            .map(|index| Ok(self.device.upload(&sealed, index)?.into()))
            .collect::<Result<Vec<HttpCall>, SessionError>>()?;
        let blob = URL_SAFE_NO_PAD.encode(sealed.blob.0);
        self.prepared.insert(sealed.blob.0, sealed);
        Ok(PreparedFile { blob, calls })
    }

    /// Sends a file already uploaded with [`Session::prepare_file`] into
    /// `chat`, described by the rest.
    #[allow(clippy::too_many_arguments)]
    pub fn send_file(
        &mut self,
        chat: String,
        blob: String,
        name: String,
        mime: String,
        width: Option<u32>,
        height: Option<u32>,
        thumb: Option<Vec<u8>>,
        duration_ms: Option<u32>,
    ) -> Result<(), SessionError> {
        let id = blob_id(&blob)?;
        let sealed = self
            .prepared
            .get(&id.0)
            .ok_or_else(|| failure("unknown_blob", "prepare the file first"))?;
        let file = FileRef {
            blob: id.0,
            key: *sealed.key,
            size: sealed.size,
            name,
            mime,
            width,
            height,
            thumb,
            duration_ms,
        };
        let chat_id = self.find_chat(&chat)?;
        let message = AppMessage::new(MessageId::random()?, now_ms(), Body::File(Box::new(file)));
        self.device.send(&chat_id, &message)?;
        self.prepared.remove(&id.0);
        Ok(())
    }

    /// The calls that fetch the file in message `message` of `chat`.
    pub fn download_calls(
        &self,
        chat: String,
        message: String,
    ) -> Result<Vec<HttpCall>, SessionError> {
        let file = self.file(&chat, &message)?;
        Ok((0..attachment::chunk_count(file.size))
            .map(|index| attachment::download(BlobId(file.blob), index).into())
            .collect())
    }

    /// Opens the file in message `message` of `chat` from the answers to
    /// [`Session::download_calls`], in order.
    pub fn open_file(
        &self,
        chat: String,
        message: String,
        answers: Vec<HttpAnswer>,
    ) -> Result<Vec<u8>, SessionError> {
        let file = self.file(&chat, &message)?;
        let chunks = answers
            .iter()
            .map(|answer| attachment::downloaded(answer.status, &answer.body))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(attachment::open_file(&file.key, file.size, &chunks)?)
    }

    fn file(&self, chat: &str, message: &str) -> Result<FileRef, SessionError> {
        let chat = self.find_chat(chat)?;
        let stored = self
            .device
            .message(&chat, message_id(message)?)?
            .ok_or_else(|| failure("unknown_message", "no such message"))?;
        match stored.content {
            StoredContent::File(file) if !stored.deleted => Ok(*file),
            _ => Err(failure("not_a_file", "that message holds no file")),
        }
    }

    /// Every chat, most recently active first.
    pub fn chats(&self) -> Result<Vec<ChatPreview>, SessionError> {
        self.device
            .chat_list()?
            .iter()
            .map(|summary| self.preview(summary))
            .collect()
    }

    /// Up to `limit` messages of `chat`, newest first, older than `before`
    /// when given.
    pub fn messages(
        &self,
        chat: String,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<MessageView>, SessionError> {
        let chat = self.find_chat(&chat)?;
        let others = self.device.others_in(&chat)?;
        self.device
            .history(&chat, before, limit)?
            .iter()
            .map(|entry| self.view(&chat, entry, others))
            .collect()
    }

    /// `@name` for `account`, or "Someone new" when the name is unknown.
    fn name_of(&self, account: zelamar_core::identity::AccountId) -> String {
        self.device
            .contacts()
            .into_iter()
            .find(|contact| contact.account == account)
            .and_then(|contact| contact.username)
            .map_or_else(|| "Someone new".to_owned(), |name| format!("@{name}"))
    }

    /// The notification for a new text from `account` in `chat`, if it is
    /// one. In a named group the title is the group, and the text says who.
    fn notice(
        &self,
        chat: &ChatId,
        account: zelamar_core::identity::AccountId,
        body: &Body,
    ) -> Option<Notice> {
        let line;
        let text = match body {
            Body::Text { text } | Body::Reply { text, .. } => text,
            Body::File(file) => {
                line = file_line(file);
                &line
            }
            Body::CallEnd { call, reason, .. } => {
                line = self.missed_call(chat, *call, *reason)?;
                &line
            }
            _ => return None,
        };
        let author = self.name_of(account);
        Some(match self.device.chat_name(chat) {
            Some(group) => Notice {
                title: group.to_owned(),
                body: shorten(&format!("{author}: {text}"), NOTICE_CHARS),
            },
            None => Notice {
                title: author,
                body: shorten(text, NOTICE_CHARS),
            },
        })
    }

    fn preview(&self, summary: &ChatSummary) -> Result<ChatPreview, SessionError> {
        Ok(ChatPreview {
            id: chat_key(&summary.chat),
            title: title(summary),
            verified: !summary.members.is_empty() && summary.members.iter().all(|m| m.verified),
            last: summary
                .last
                .as_ref()
                .map(|entry| self.view(&summary.chat, entry, summary.members.len()))
                .transpose()?,
            unread: summary.unread,
            request: summary.request,
            group: summary.name.is_some() || summary.members.len() > 1,
        })
    }

    /// One message as the chat screen shows it, with its quote and
    /// reactions.
    /// `others` is how many other accounts are in the chat: in a group, a
    /// message counts as delivered or read once all of them got that far.
    fn view(
        &self,
        chat: &ChatId,
        entry: &HistoryEntry,
        others: usize,
    ) -> Result<MessageView, SessionError> {
        let message = &entry.message;
        let (text, unsupported, answers) = match &message.content {
            StoredContent::Text { text } => (text.clone(), false, None),
            StoredContent::Reply { text, to } => (text.clone(), false, Some(*to)),
            StoredContent::Unknown { .. }
            | StoredContent::Named { .. }
            | StoredContent::Left
            | StoredContent::File(_)
            | StoredContent::Call(_) => (
                String::new(),
                matches!(message.content, StoredContent::Unknown { .. }),
                None,
            ),
        };
        let file = match &message.content {
            StoredContent::File(file) => Some(FileView {
                name: file.name.clone(),
                mime: file.mime.clone(),
                size: file.size,
                width: file.width,
                height: file.height,
                thumb: file.thumb.clone(),
                duration_ms: file.duration_ms,
            }),
            _ => None,
        };
        let event = match &message.content {
            StoredContent::Named { name } => Some(ChatEvent {
                kind: ChatEventKind::Named,
                name: Some(name.clone()),
            }),
            StoredContent::Left => Some(ChatEvent {
                kind: ChatEventKind::Left,
                name: None,
            }),
            _ => None,
        };
        let call = match &message.content {
            StoredContent::Call(record) => Some(call_view(record, message.sent_at_ms, now_ms())),
            _ => None,
        };
        let reply_to = answers
            .map(|to| {
                let quoted = self.device.message(chat, to)?;
                Ok::<_, SessionError>(Quote {
                    id: URL_SAFE_NO_PAD.encode(to.0),
                    text: match &quoted {
                        None => "Message unavailable".to_owned(),
                        Some(quoted) if quoted.deleted => "Message deleted".to_owned(),
                        Some(quoted) => match &quoted.content {
                            StoredContent::Text { text } | StoredContent::Reply { text, .. } => {
                                shorten(text, QUOTE_CHARS)
                            }
                            StoredContent::File(file) => file_line(file),
                            StoredContent::Call(call) => call_line(call.video).to_owned(),
                            StoredContent::Unknown { .. }
                            | StoredContent::Named { .. }
                            | StoredContent::Left => "Message unavailable".to_owned(),
                        },
                    },
                    outgoing: quoted.is_some_and(|quoted| quoted.outgoing),
                })
            })
            .transpose()?;
        let own = self.device.account();
        let mut reactions: Vec<Reaction> = Vec::new();
        for (account, emoji) in &message.reactions {
            match reactions.iter_mut().find(|r| &r.emoji == emoji) {
                Some(reaction) => {
                    reaction.count += 1;
                    reaction.mine |= *account == own;
                }
                None => reactions.push(Reaction {
                    emoji: emoji.clone(),
                    count: 1,
                    mine: *account == own,
                }),
            }
        }
        reactions.sort_by_key(|reaction| std::cmp::Reverse(reaction.count));
        let status = delivery_status(
            message.read_by.len(),
            message.delivered_to.len(),
            message.status,
            others,
        );
        Ok(MessageView {
            id: URL_SAFE_NO_PAD.encode(message.id.0),
            seq: entry.seq,
            outgoing: message.outgoing,
            text,
            deleted: message.deleted,
            unsupported,
            edited: message.edited_at_ms.is_some(),
            sent_at_ms: i64::try_from(message.sent_at_ms).unwrap_or(i64::MAX),
            status,
            reply_to,
            reactions,
            editable: message.outgoing
                && message.author.account == own
                && !message.deleted
                && !unsupported
                && event.is_none()
                && call.is_none(),
            author: if message.author.account == own {
                "You".to_owned()
            } else {
                member_title(
                    &self
                        .device
                        .contacts()
                        .into_iter()
                        .find(|contact| contact.account == message.author.account)
                        .unwrap_or_else(|| Contact::unknown(message.author.account)),
                )
            },
            event,
            file,
            call,
        })
    }

    /// What `message`, from `author` in `chat`, asks of this device if it
    /// is about a call (ADR-0019).
    fn call_signal(
        &self,
        chat: &ChatId,
        author: Author,
        message: &AppMessage,
    ) -> Option<CallSignal> {
        let own = author.account == self.device.account();
        let signal = |kind, call: MessageId| CallSignal {
            kind,
            chat: chat_key(chat),
            call: URL_SAFE_NO_PAD.encode(call.0),
            own,
            peer: self.peer_of(chat),
            video: false,
            sdp: None,
            ring: false,
            ending: None,
            duration_ms: None,
        };
        match &message.body {
            Body::Call { video, sdp } => {
                let record = self.call_record(chat, message.id)?;
                let StoredContent::Call(details) = &record.content else {
                    return None;
                };
                let ring = !own
                    && self.device.accepted(chat)
                    && self.device.others_in(chat).ok() == Some(1)
                    && details.answered_by.is_none()
                    && details.end.is_none()
                    && now_ms().saturating_sub(message.sent_at_ms) <= RING_WINDOW_MS;
                Some(CallSignal {
                    video: *video,
                    sdp: Some(sdp.clone()),
                    ring,
                    ..signal(CallSignalKind::Offer, message.id)
                })
            }
            Body::CallAnswer { call, sdp } => Some(CallSignal {
                sdp: Some(sdp.clone()),
                ..signal(CallSignalKind::Answer, *call)
            }),
            Body::CallEnd {
                call,
                reason,
                duration_ms,
            } => Some(CallSignal {
                ending: Some(ending_of(*reason)),
                duration_ms: *duration_ms,
                ..signal(CallSignalKind::End, *call)
            }),
            Body::Receipt {
                status: ReceiptStatus::Delivered,
                messages,
            } if !own => messages
                .iter()
                .copied()
                .find(|id| {
                    self.call_record(chat, *id).is_some_and(|record| {
                        record.outgoing
                            && matches!(&record.content, StoredContent::Call(call)
                                if call.answered_by.is_none() && call.end.is_none())
                    })
                })
                .map(|id| signal(CallSignalKind::Ringing, id)),
            _ => None,
        }
    }

    /// The call `id` of `chat`, if history has it.
    fn call_record(&self, chat: &ChatId, id: MessageId) -> Option<StoredMessage> {
        self.device
            .message(chat, id)
            .ok()
            .flatten()
            .filter(|record| matches!(record.content, StoredContent::Call(_)))
    }

    /// "Missed call" when `call`, ended for `reason` by the caller, rang
    /// here and nobody on this account picked up.
    fn missed_call(&self, chat: &ChatId, call: MessageId, reason: CallEndReason) -> Option<String> {
        let record = self.call_record(chat, call)?;
        let StoredContent::Call(details) = &record.content else {
            return None;
        };
        let missed = !record.outgoing
            && details.answered_by.is_none()
            && reason != CallEndReason::Busy
            && self.device.accepted(chat);
        missed.then(|| {
            if details.video {
                "Missed video call".to_owned()
            } else {
                "Missed call".to_owned()
            }
        })
    }

    /// The other person in `chat`, as a title.
    fn peer_of(&self, chat: &ChatId) -> String {
        self.device
            .chat_list()
            .ok()
            .and_then(|list| list.into_iter().find(|summary| &summary.chat == chat))
            .map_or_else(|| "Someone".to_owned(), |summary| title(&summary))
    }

    fn find_chat(&self, key: &str) -> Result<ChatId, SessionError> {
        self.device
            .chats()
            .into_iter()
            .find(|chat| chat_key(chat) == key)
            .ok_or_else(|| failure("unknown_chat", "no such chat on this device"))
    }
}

fn store_path(directory: &str) -> PathBuf {
    PathBuf::from(directory).join(STORE_FILE)
}

fn master(bytes: &[u8]) -> Result<MasterKey, SessionError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| failure("bad_master_key", "a master key is 32 bytes"))?;
    Ok(MasterKey::from_bytes(bytes))
}

fn chat_key(chat: &ChatId) -> String {
    URL_SAFE_NO_PAD.encode(chat.as_bytes())
}

/// How far an outgoing message got: read or delivered once every other
/// account in the chat got that far, else sent or still queued.
fn delivery_status(
    read: usize,
    delivered: usize,
    sent: SendStatus,
    others: usize,
) -> MessageStatus {
    if others > 0 && read >= others {
        MessageStatus::Read
    } else if others > 0 && delivered >= others {
        MessageStatus::Delivered
    } else if sent == SendStatus::Sent {
        MessageStatus::Sent
    } else {
        MessageStatus::Queued
    }
}

/// A file in one line, for quotes and notifications.
fn file_line(file: &FileRef) -> String {
    if file.mime.starts_with("image/") {
        "📷 Photo".to_owned()
    } else if file.mime.starts_with("audio/") && file.duration_ms.is_some() {
        "🎤 Voice message".to_owned()
    } else {
        format!("📎 {}", file.name)
    }
}

fn blob_id(text: &str) -> Result<BlobId, SessionError> {
    URL_SAFE_NO_PAD
        .decode(text)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .map(BlobId)
        .ok_or_else(|| failure("malformed", "not a blob id"))
}

/// The directory entries behind `invitees`.
fn entries(invitees: &[Invitee]) -> Result<Vec<service::DirectoryEntry>, SessionError> {
    invitees
        .iter()
        .map(|invitee| {
            Ok(service::directory_entry(
                &invitee.username,
                invitee.answer.status,
                &invitee.answer.body,
            )?)
        })
        .collect()
}

fn device_key(device: &DeviceId) -> String {
    URL_SAFE_NO_PAD.encode(device.as_bytes())
}

fn device_id(text: &str) -> Result<DeviceId, SessionError> {
    URL_SAFE_NO_PAD
        .decode(text)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .map(DeviceId::from_bytes)
        .ok_or_else(|| failure("malformed", "not a device id"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `@bob`, `@alice, @bob`, or a short fingerprint for an unknown account.
fn title(summary: &ChatSummary) -> String {
    if let Some(name) = &summary.name {
        return name.clone();
    }
    if summary.members.is_empty() {
        return "Just you".to_owned();
    }
    summary
        .members
        .iter()
        .map(member_title)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `@bob`, or a short fingerprint for an account without a known name.
fn member_title(member: &Contact) -> String {
    match &member.username {
        Some(name) => format!("@{name}"),
        None => {
            let hex: String = member.account.as_bytes()[..4]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("Unknown {hex}")
        }
    }
}

/// `text` cut to `limit` characters, with an ellipsis when cut.
fn shorten(text: &str, limit: usize) -> String {
    let mut out: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        out.push('…');
    }
    out
}

/// The servers in the answer to [`Session::ice_call`].
pub fn ice_servers(status: u16, body: Vec<u8>) -> Result<Vec<IceServerView>, SessionError> {
    Ok(service::ice_servers(status, &body)?
        .into_iter()
        .map(|server| IceServerView {
            urls: server.urls,
            username: server.username,
            credential: server.credential,
        })
        .collect())
}

fn ending_of(reason: CallEndReason) -> CallEnding {
    match reason {
        CallEndReason::Hangup => CallEnding::Hangup,
        CallEndReason::Declined => CallEnding::Declined,
        CallEndReason::Missed => CallEnding::Missed,
        CallEndReason::Busy => CallEnding::Busy,
        CallEndReason::Failed => CallEnding::Failed,
        _ => CallEnding::Other,
    }
}

/// How a call looks in its chat, at `now`; `sent_at` is when it started.
fn call_view(record: &CallRecord, sent_at: u64, now: u64) -> CallView {
    let answered = record.answered_by.is_some();
    let outcome = match record.end {
        None if now.saturating_sub(sent_at) < RING_WINDOW_MS => CallOutcome::Ongoing,
        None if answered && now.saturating_sub(sent_at) < CALL_FORGOTTEN_MS => CallOutcome::Ongoing,
        // Nobody said how it ended: picked up means talked, else missed.
        None => {
            if answered {
                CallOutcome::Talked
            } else {
                CallOutcome::Missed
            }
        }
        Some(_) if answered => CallOutcome::Talked,
        Some(CallEndReason::Hangup) => CallOutcome::Cancelled,
        Some(CallEndReason::Declined) => CallOutcome::Declined,
        Some(CallEndReason::Busy) => CallOutcome::Busy,
        Some(CallEndReason::Failed) => CallOutcome::Failed,
        Some(_) => CallOutcome::Missed,
    };
    CallView {
        video: record.video,
        outcome,
        duration_ms: record.duration_ms,
    }
}

/// A call, in a quote or a notification.
const fn call_line(video: bool) -> &'static str {
    if video {
        "📹 Video call"
    } else {
        "📞 Call"
    }
}

/// A local-network nonce from its bytes.
fn lan_nonce(nonce: Vec<u8>) -> Result<[u8; lan::NONCE_LEN], SessionError> {
    nonce
        .try_into()
        .map_err(|_| failure("malformed", "a nonce is 16 bytes"))
}

/// A message id from its base64url form.
fn message_id(text: &str) -> Result<MessageId, SessionError> {
    URL_SAFE_NO_PAD
        .decode(text)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .map(MessageId)
        .ok_or_else(|| failure("malformed", "not a message id"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// A stand-in delivery service: just enough routing for two sessions.
    #[derive(Default)]
    struct Stub {
        names: HashMap<String, String>,
        registrations: HashMap<String, (String, Vec<String>)>,
        mailboxes: HashMap<String, Vec<Vec<u8>>>,
        links: HashMap<String, (Vec<u8>, Option<Vec<u8>>)>,
    }

    fn json(value: &serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(value).expect("json")
    }

    impl Stub {
        fn call(&mut self, call: &HttpCall) -> (u16, Vec<u8>) {
            let parts: Vec<&str> = call.path.split('/').skip(2).collect();
            let body = || serde_json::from_slice::<serde_json::Value>(&call.body).expect("json");
            match (call.method.as_str(), parts.as_slice()) {
                ("POST", ["accounts"]) => {
                    let request = body();
                    let name = request["username"].as_str().expect("name").to_owned();
                    let key = request["account"].as_str().expect("key").to_owned();
                    self.names.insert(name.clone(), key);
                    (201, json(&serde_json::json!({ "username": name })))
                }
                ("PUT", ["accounts", name, "devices", _]) => {
                    let request = body();
                    let certificate = call
                        .headers
                        .iter()
                        .find(|h| h.name == "zelamar-certificate")
                        .expect("certificate")
                        .value
                        .clone();
                    let packages = request["key_packages"]
                        .as_array()
                        .expect("key packages")
                        .iter()
                        .map(|kp| kp.as_str().expect("string").to_owned())
                        .collect::<Vec<_>>();
                    let mailbox = request["mailbox"].as_str().expect("mailbox").to_owned();
                    self.mailboxes.entry(mailbox).or_default();
                    let count = packages.len();
                    self.registrations
                        .insert((*name).to_owned(), (certificate, packages));
                    (200, json(&serde_json::json!({ "key_packages": count })))
                }
                ("GET", ["accounts", name]) => {
                    let (certificate, packages) =
                        self.registrations.get_mut(*name).expect("registered");
                    let key_package = packages.remove(0);
                    let answer = serde_json::json!({
                        "username": name,
                        "account": self.names[*name],
                        "devices": [{ "certificate": certificate, "key_package": key_package }],
                    });
                    (200, json(&answer))
                }
                ("GET", ["accounts", name, "key"]) => (
                    200,
                    json(&serde_json::json!({ "username": name, "account": self.names[*name] })),
                ),
                ("POST", ["deliver"]) => {
                    let frame = &call.body;
                    let count = usize::from(frame[19]);
                    let payload = &frame[20 + 32 * count + 4..];
                    for at in 0..count {
                        let to = &frame[20 + 32 * at..20 + 32 * (at + 1)];
                        let mut envelope = b"ZE\0".to_vec();
                        envelope.extend_from_slice(&frame[3..19]);
                        envelope.extend_from_slice(to);
                        envelope.extend_from_slice(
                            &u32::try_from(payload.len()).expect("len").to_be_bytes(),
                        );
                        envelope.extend_from_slice(payload);
                        self.mailboxes
                            .get_mut(&URL_SAFE_NO_PAD.encode(to))
                            .expect("mailbox")
                            .push(envelope);
                    }
                    (202, json(&serde_json::json!({ "delivered": count })))
                }
                ("PUT", ["links", slot, "request"]) => {
                    self.links
                        .insert((*slot).to_owned(), (call.body.clone(), None));
                    (201, json(&serde_json::json!({})))
                }
                ("PUT", ["links", slot, "answer"]) => {
                    self.links.get_mut(*slot).expect("slot").1 = Some(call.body.clone());
                    (201, json(&serde_json::json!({})))
                }
                ("GET", ["links", slot]) => match self.links.get(*slot) {
                    None => (404, json(&serde_json::json!({ "error": "unknown_link" }))),
                    Some((request, answer)) => (
                        200,
                        json(&serde_json::json!({
                            "request": URL_SAFE_NO_PAD.encode(request),
                            "answer": answer.as_ref().map(|a| URL_SAFE_NO_PAD.encode(a)),
                        })),
                    ),
                },
                other => panic!("unrouted {other:?}"),
            }
        }

        fn drain(&mut self, session: &Session) -> Vec<Vec<u8>> {
            let path = session.mailbox().path;
            let id = path.rsplit('/').next().expect("id").to_owned();
            std::mem::take(self.mailboxes.get_mut(&id).expect("mailbox"))
        }
    }

    fn person(stub: &mut Stub, dir: &tempfile::TempDir, name: &str) -> Session {
        let directory = dir.path().join(name).to_string_lossy().into_owned();
        let mut session =
            Session::create(directory.clone(), new_master_key().expect("key")).expect("create");
        assert!(session_exists(directory));
        let claim = session.claim_with(name, "invite", 4).expect("claim");
        let (status, body) = stub.call(&claim);
        assert_eq!(session.claimed(status, body).expect("claimed"), name);
        let registration = session.register().expect("register");
        let (status, body) = stub.call(&registration);
        assert!(session.registered(status, body).expect("registered") > 0);
        session
    }

    fn flush(stub: &mut Stub, session: &mut Session) {
        for pending in session.pending_sends().expect("pending") {
            let (status, body) = stub.call(&pending.call);
            session.sent(pending.id, status, body).expect("sent");
        }
    }

    fn read(stub: &mut Stub, session: &mut Session) -> Vec<InboxEvent> {
        stub.drain(session)
            .into_iter()
            .map(|bytes| session.open_envelope(bytes).expect("envelope"))
            .collect()
    }

    #[test]
    fn a_group_message_is_read_once_everyone_read_it() {
        let status =
            |read, delivered, others| delivery_status(read, delivered, SendStatus::Sent, others);
        assert_eq!(status(1, 1, 1), MessageStatus::Read, "a chat for two");
        assert_eq!(
            status(1, 3, 3),
            MessageStatus::Delivered,
            "one of three read"
        );
        assert_eq!(status(3, 3, 3), MessageStatus::Read);
        assert_eq!(
            status(0, 2, 3),
            MessageStatus::Sent,
            "one still has not got it"
        );
        assert_eq!(status(0, 0, 0), MessageStatus::Sent, "just you");
        assert_eq!(
            delivery_status(0, 0, SendStatus::Queued, 2),
            MessageStatus::Queued
        );
    }

    #[test]
    fn a_new_device_joins_the_account_through_the_app_api() {
        let dir = tempfile::tempdir().expect("dir");
        let mut stub = Stub::default();
        let mut alice = person(&mut stub, &dir, "alice");
        assert!(alice.can_link());
        let code = alice.offer_link().expect("offer");
        let (status, body) = stub.call(&alice.link_check_call().expect("check"));
        assert!(alice.link_applicant(status, body).expect("slot").is_none());

        let mut joiner = Joiner::new(code.to_uppercase(), "Android phone".into()).expect("code");
        let (status, body) = stub.call(&joiner.request_call().expect("request"));
        joiner.requested(status, body).expect("requested");
        let directory = dir.path().join("phone").to_string_lossy().into_owned();
        let key = new_master_key().expect("key");
        let (status, body) = stub.call(&joiner.check_call().expect("check"));
        assert!(joiner
            .finish(status, body, directory.clone(), key.clone())
            .expect("waiting")
            .is_none());

        let (status, body) = stub.call(&alice.link_check_call().expect("check"));
        let applicant = alice
            .link_applicant(status, body)
            .expect("applicant")
            .expect("someone asked");
        assert_eq!(applicant.label, "Android phone");
        assert_eq!(applicant.match_code, joiner.match_code().expect("match"));
        let (status, body) = stub.call(&alice.approve_link().expect("approve"));
        let device = alice.link_approved(status, body).expect("approved");
        assert_eq!(device.len(), 22);
        assert!(alice.link_check_call().is_err(), "the offer is over");

        let (status, body) = stub.call(&joiner.check_call().expect("check"));
        let phone = joiner
            .finish(status, body, directory.clone(), key)
            .expect("finish")
            .expect("linked");
        assert_eq!(phone.username().as_deref(), Some("alice"));
        assert_eq!(phone.fingerprint(), alice.fingerprint());
        assert!(!phone.can_link());
        assert!(session_exists(directory));
        assert!(joiner.check_call().is_err(), "a joiner is used once");
        let mut phone = phone;
        assert!(phone.offer_link().is_err());
    }

    #[test]
    fn two_sessions_meet_and_talk_through_the_app_api() {
        let dir = tempfile::tempdir().expect("dir");
        let mut stub = Stub::default();
        let mut alice = person(&mut stub, &dir, "alice");
        let mut bob = person(&mut stub, &dir, "bob");

        let (status, body) = stub.call(&lookup_call("Bob".into()).expect("lookup"));
        let chat = alice
            .start_chat("bob".into(), status, body, None)
            .expect("chat");
        alice
            .send_text(chat.clone(), "Hello Bob".into())
            .expect("send");
        let queued = alice.messages(chat.clone(), None, 10).expect("history");
        assert_eq!(queued[0].status, MessageStatus::Queued);
        flush(&mut stub, &mut alice);
        assert_eq!(
            alice.messages(chat.clone(), None, 10).expect("history")[0].status,
            MessageStatus::Sent
        );

        let events = read(&mut stub, &mut bob);
        assert!(events[0].joined);
        // Bob does not know Alice's name yet when the request arrives.
        let request = events[0].notice.clone().expect("a notice for the request");
        assert_eq!(request.title, "Someone new");
        assert_eq!(request.body, "New chat request");
        let notice = events
            .iter()
            .skip(1)
            .find_map(|event| event.notice.clone())
            .expect("a notice for Alice's text");
        assert_eq!(notice.title, "@alice");
        assert_eq!(notice.body, "Hello Bob");
        assert_eq!(events.iter().filter(|e| e.notice.is_some()).count(), 2);
        assert!(events
            .iter()
            .all(|event| event.ack.is_some() && event.refused.is_none()));
        assert_eq!(
            events.last().and_then(|e| e.chat.clone()),
            Some(chat.clone())
        );

        // Bob has Alice's word for her name until the directory confirms it.
        let before = bob.chats().expect("chats");
        assert_eq!(before[0].title, "@alice");
        assert!(!before[0].verified);
        assert_eq!(bob.names_to_confirm(), vec!["alice".to_owned()]);
        let (status, body) = stub.call(&key_lookup_call("alice".into()).expect("key lookup"));
        assert!(bob
            .confirm_name("alice".into(), status, body)
            .expect("confirm"));
        let after = bob.chats().expect("chats");
        assert!(after[0].verified);
        let last = after[0].last.clone().expect("last message");
        assert_eq!(last.text, "Hello Bob");
        assert!(!last.outgoing);
        assert_eq!(after[0].unread, 1);
        assert!(after[0].request, "Bob has not answered yet");
        bob.accept_chat(chat.clone()).expect("accept");
        assert!(!bob.chats().expect("chats")[0].request);
        assert_eq!(bob.mark_read(chat.clone()).expect("read"), 1);
        assert_eq!(bob.chats().expect("chats")[0].unread, 0);

        assert_eq!(alice.chats().expect("chats")[0].title, "@bob");
        // Both sides see the same safety code.
        let code = alice
            .safety_code(chat.clone())
            .expect("code")
            .expect("one other person");
        assert_eq!(code.len(), 12);
        assert!(code.iter().all(|group| group.len() == 5));
        assert_eq!(bob.safety_code(chat.clone()).expect("code"), Some(code));
        assert_eq!(alice.fingerprint().len(), "a1b2 ".len() * 8 - 1);
        assert_ne!(alice.fingerprint(), bob.fingerprint());
        alice.delete_chat(chat).expect("delete");
        assert!(alice.chats().expect("chats").is_empty());
    }

    /// The call signals in `events`.
    fn signals(events: &[InboxEvent]) -> Vec<CallSignal> {
        events
            .iter()
            .filter_map(|event| event.call.clone())
            .collect()
    }

    #[test]
    fn a_call_rings_connects_and_ends_through_the_app_api() {
        let dir = tempfile::tempdir().expect("dir");
        let mut stub = Stub::default();
        let mut alice = person(&mut stub, &dir, "alice");
        let mut bob = person(&mut stub, &dir, "bob");
        let (status, body) = stub.call(&lookup_call("bob".into()).expect("lookup"));
        let chat = alice
            .start_chat("bob".into(), status, body, None)
            .expect("chat");
        alice.send_text(chat.clone(), "Free?".into()).expect("send");
        flush(&mut stub, &mut alice);
        read(&mut stub, &mut bob);

        // Nothing rings from a chat request.
        let call = alice
            .start_call(chat.clone(), false, "offer".into())
            .expect("call");
        flush(&mut stub, &mut alice);
        let offer = signals(&read(&mut stub, &mut bob)).remove(0);
        assert_eq!(offer.kind, CallSignalKind::Offer);
        assert!(!offer.ring, "Bob has not accepted the chat");
        alice
            .end_call(chat.clone(), call, CallEnding::Missed, None)
            .expect("end");
        flush(&mut stub, &mut alice);
        let events = read(&mut stub, &mut bob);
        assert!(
            events.iter().all(|event| event.notice.is_none()),
            "a stranger's call is no news"
        );

        bob.accept_chat(chat.clone()).expect("accept");
        flush(&mut stub, &mut bob);
        read(&mut stub, &mut alice);

        let call = alice
            .start_call(chat.clone(), true, "offer".into())
            .expect("call");
        flush(&mut stub, &mut alice);
        let offer = signals(&read(&mut stub, &mut bob)).remove(0);
        assert_eq!(
            (offer.kind, offer.ring, offer.own, offer.video),
            (CallSignalKind::Offer, true, false, true)
        );
        assert_eq!(offer.call, call);
        assert_eq!(offer.peer, "@alice");
        assert_eq!(offer.sdp.as_deref(), Some("offer"));
        let ringing = bob.chats().expect("chats")[0].last.clone().expect("last");
        let line = ringing.call.expect("a call line");
        assert_eq!((line.outcome, line.video), (CallOutcome::Ongoing, true));
        assert!(!ringing.editable);

        // Bob's device says it got the call: Alice hears it ring.
        flush(&mut stub, &mut bob);
        let ringing = signals(&read(&mut stub, &mut alice));
        assert_eq!(ringing[0].kind, CallSignalKind::Ringing);
        assert_eq!(ringing[0].call, call);

        bob.answer_call(chat.clone(), call.clone(), "answer".into())
            .expect("answer");
        flush(&mut stub, &mut bob);
        let answer = signals(&read(&mut stub, &mut alice)).remove(0);
        assert_eq!(
            (answer.kind, answer.own, answer.sdp.as_deref()),
            (CallSignalKind::Answer, false, Some("answer"))
        );

        alice
            .end_call(chat.clone(), call, CallEnding::Hangup, Some(42_000))
            .expect("hang up");
        flush(&mut stub, &mut alice);
        let events = read(&mut stub, &mut bob);
        let end = signals(&events).remove(0);
        assert_eq!(
            (end.kind, end.ending, end.duration_ms),
            (CallSignalKind::End, Some(CallEnding::Hangup), Some(42_000))
        );
        assert!(events.iter().all(|event| event.notice.is_none()));
        let talked = bob.messages(chat.clone(), None, 1).expect("history")[0]
            .call
            .clone()
            .expect("call");
        assert_eq!(
            (talked.outcome, talked.duration_ms),
            (CallOutcome::Talked, Some(42_000))
        );
        bob.mark_read(chat.clone()).expect("read");

        // A call nobody picks up: missed, and Bob is told.
        let call = alice
            .start_call(chat.clone(), false, "offer".into())
            .expect("call");
        alice
            .end_call(chat.clone(), call, CallEnding::Missed, None)
            .expect("give up");
        flush(&mut stub, &mut alice);
        let events = read(&mut stub, &mut bob);
        let notice = events
            .iter()
            .find_map(|event| event.notice.clone())
            .expect("a notice");
        assert_eq!(
            (notice.title.as_str(), notice.body.as_str()),
            ("@alice", "Missed call")
        );
        let summary = bob.chats().expect("chats")[0].clone();
        assert_eq!(summary.unread, 1);
        assert_eq!(
            summary.last.and_then(|last| last.call).map(|c| c.outcome),
            Some(CallOutcome::Missed)
        );
        assert!(alice.start_call("nope".into(), false, "x".into()).is_err());
    }

    #[test]
    fn a_call_line_says_how_the_call_went() {
        let bob = Author {
            account: AccountKey::generate().expect("account").id(),
            device: DeviceId::random().expect("device"),
        };
        let record = |answered: bool, end| CallRecord {
            video: false,
            answered_by: answered.then_some(bob),
            end,
            duration_ms: answered.then_some(5_000),
        };
        let at = |record: &CallRecord, age| call_view(record, 1_000_000, 1_000_000 + age).outcome;
        assert_eq!(at(&record(false, None), 1_000), CallOutcome::Ongoing);
        assert_eq!(
            at(&record(false, None), RING_WINDOW_MS),
            CallOutcome::Missed
        );
        assert_eq!(
            at(&record(true, None), 60 * 60 * 1000),
            CallOutcome::Ongoing
        );
        assert_eq!(
            at(&record(true, None), CALL_FORGOTTEN_MS),
            CallOutcome::Talked
        );
        assert_eq!(
            at(&record(false, Some(CallEndReason::Hangup)), 1),
            CallOutcome::Cancelled
        );
        assert_eq!(
            at(&record(true, Some(CallEndReason::Failed)), 1),
            CallOutcome::Talked
        );
        for (reason, outcome) in [
            (CallEndReason::Declined, CallOutcome::Declined),
            (CallEndReason::Missed, CallOutcome::Missed),
            (CallEndReason::Busy, CallOutcome::Busy),
            (CallEndReason::Failed, CallOutcome::Failed),
            (CallEndReason::Other, CallOutcome::Missed),
        ] {
            assert_eq!(at(&record(false, Some(reason)), 1), outcome);
        }
    }

    #[test]
    fn replies_reactions_edits_and_deletions_reach_the_other_side() {
        let dir = tempfile::tempdir().expect("dir");
        let mut stub = Stub::default();
        let mut alice = person(&mut stub, &dir, "alice");
        let mut bob = person(&mut stub, &dir, "bob");
        let (status, body) = stub.call(&lookup_call("bob".into()).expect("lookup"));
        let chat = alice
            .start_chat("bob".into(), status, body, None)
            .expect("chat");
        alice
            .send_text(chat.clone(), "Dinner?".into())
            .expect("send");
        flush(&mut stub, &mut alice);
        read(&mut stub, &mut bob);

        let question = bob.messages(chat.clone(), None, 10).expect("history")[0].clone();
        assert!(!question.editable, "Alice's words are not Bob's to edit");
        bob.reply_text(chat.clone(), question.id.clone(), "Yes!".into())
            .expect("reply");
        bob.react(chat.clone(), question.id.clone(), Some("❤️".into()))
            .expect("react");
        // An edit of someone else's message changes nothing anywhere.
        bob.edit_text(chat.clone(), question.id.clone(), "Forged".into())
            .expect("send");
        flush(&mut stub, &mut bob);
        read(&mut stub, &mut alice);

        let seen = alice.messages(chat.clone(), None, 10).expect("history");
        let answer = &seen[0];
        assert_eq!(answer.text, "Yes!");
        let quote = answer.reply_to.clone().expect("a quote");
        assert_eq!((quote.text.as_str(), quote.outgoing), ("Dinner?", true));
        let original = seen.iter().find(|m| m.id == question.id).expect("original");
        assert_eq!(original.text, "Dinner?");
        assert!(original.editable);
        assert_eq!(
            original
                .reactions
                .iter()
                .map(|r| (r.emoji.as_str(), r.count, r.mine))
                .collect::<Vec<_>>(),
            vec![("❤️", 1, false)]
        );

        alice
            .edit_text(chat.clone(), question.id.clone(), "Dinner at eight?".into())
            .expect("edit");
        alice
            .delete_message(chat.clone(), question.id.clone())
            .expect("delete");
        flush(&mut stub, &mut alice);
        read(&mut stub, &mut bob);
        let after = bob.messages(chat, None, 10).expect("history");
        let gone = after
            .iter()
            .find(|m| m.id == question.id)
            .expect("original");
        assert!(gone.deleted);
        assert_eq!(gone.text, "");
        assert_eq!(
            after[0].reply_to.clone().expect("quote").text,
            "Message deleted"
        );
        assert_eq!(
            message_id("not an id").expect_err("refused").code,
            "malformed"
        );
    }

    #[test]
    fn errors_carry_stable_codes() {
        let dir = tempfile::tempdir().expect("dir");
        let directory = dir.path().join("x").to_string_lossy().into_owned();
        let error = Session::create(directory.clone(), vec![0; 31])
            .err()
            .expect("short key refused");
        assert_eq!(error.code, "bad_master_key");
        assert_eq!(
            lookup_call("no".into()).expect_err("bad name").code,
            "malformed"
        );

        let session = Session::create(directory.clone(), vec![7; 32]).expect("create");
        drop(session);
        let wrong = Session::open(directory, vec![8; 32])
            .err()
            .expect("wrong key");
        assert_eq!(wrong.code, "wrong_key");

        let refused = SessionError::from(CoreError::Service {
            status: 409,
            code: "username_taken".into(),
        });
        assert_eq!(refused.code, "username_taken");
        assert!(!refused.transient);
        assert_eq!(
            normalize_username("Alice_1".into()).as_deref(),
            Some("alice_1")
        );
    }
}
