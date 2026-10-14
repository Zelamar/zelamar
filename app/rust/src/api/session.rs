//! The app's session: one device of one account, talking to the delivery
//! service through Dart.
//!
//! Dart moves bytes (HTTPS and WebSocket); everything that signs, encrypts,
//! verifies or decides lives in the core (ADR-0012). Each method either
//! builds a request ([`HttpCall`]) or reads the answer to one.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use zelamar_core::content::{AppMessage, MessageId};
use zelamar_core::device::{ChatId, ChatSummary, Device, Received};
use zelamar_core::history::{HistoryEntry, SendStatus, StoredContent};
use zelamar_core::identity::AccountKey;
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
    /// Why the envelope was refused, if it was.
    pub refused: Option<String>,
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
}

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
}

/// One device of one account, stored encrypted in a directory.
pub struct Session {
    device: Device,
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
        Ok(Self { device })
    }

    /// Opens the session stored in `directory`.
    pub fn open(directory: String, master_key: Vec<u8>) -> Result<Self, SessionError> {
        let device = Device::open(&store_path(&directory), &master(&master_key)?)?;
        Ok(Self { device })
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

    /// Starts a chat from the answer to [`lookup_call`] for `username`.
    /// Returns the chat id.
    pub fn start_chat(
        &mut self,
        username: String,
        status: u16,
        body: Vec<u8>,
    ) -> Result<String, SessionError> {
        let entry = service::directory_entry(&username, status, &body)?;
        let (chat, _) = self.device.start_chat(&entry)?;
        Ok(chat_key(&chat))
    }

    /// Writes `text` into `chat`. It goes out with the next
    /// [`Session::pending_sends`].
    pub fn send_text(&mut self, chat: String, text: String) -> Result<(), SessionError> {
        let chat = self.find_chat(&chat)?;
        let message = AppMessage::text(MessageId::random()?, now_ms(), text);
        self.device.send(&chat, &message)?;
        Ok(())
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
    /// leaves the outbox and its message counts as sent.
    pub fn sent(&mut self, id: Vec<u8>, status: u16, body: Vec<u8>) -> Result<(), SessionError> {
        service::delivered(status, &body)?;
        let id: [u8; 16] = id
            .try_into()
            .map_err(|_| failure("malformed", "an outbox id is 16 bytes"))?;
        Ok(self
            .device
            .confirm_sent(zelamar_core::envelope::EnvelopeId(id))?)
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
            Ok(Received::Joined { chat, .. }) => InboxEvent {
                ack,
                chat: Some(chat_key(&chat)),
                joined: true,
                refused: None,
            },
            Ok(
                Received::Message { chat, .. }
                | Received::ChatUpdated { chat }
                | Received::OwnEcho { chat },
            ) => InboxEvent {
                ack,
                chat: Some(chat_key(&chat)),
                joined: false,
                refused: None,
            },
            Ok(_) => InboxEvent {
                ack,
                chat: None,
                joined: false,
                refused: None,
            },
            Err(error) => InboxEvent {
                ack,
                chat: None,
                joined: false,
                refused: Some(SessionError::from(error).code),
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
        let entry = service::directory_entry(&username, status, &body)?;
        Ok(self.device.confirm_contact(&entry)?)
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

    /// Every chat, most recently active first.
    pub fn chats(&self) -> Result<Vec<ChatPreview>, SessionError> {
        Ok(self
            .device
            .chat_list()?
            .into_iter()
            .map(|summary| preview(&summary))
            .collect())
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
        Ok(self
            .device
            .history(&chat, before, limit)?
            .iter()
            .map(message_view)
            .collect())
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn preview(summary: &ChatSummary) -> ChatPreview {
    let title = if summary.members.is_empty() {
        "Just you".to_owned()
    } else {
        summary
            .members
            .iter()
            .map(|member| match &member.username {
                Some(name) => format!("@{name}"),
                None => {
                    let hex: String = member.account.as_bytes()[..4]
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    format!("Unknown {hex}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    ChatPreview {
        id: chat_key(&summary.chat),
        title,
        verified: !summary.members.is_empty() && summary.members.iter().all(|m| m.verified),
        last: summary.last.as_ref().map(message_view),
        unread: summary.unread,
        request: summary.request,
    }
}

fn message_view(entry: &HistoryEntry) -> MessageView {
    let message = &entry.message;
    let (text, unsupported) = match &message.content {
        StoredContent::Text { text } | StoredContent::Reply { text, .. } => (text.clone(), false),
        StoredContent::Unknown { .. } => (String::new(), true),
    };
    let status = if !message.read_by.is_empty() {
        MessageStatus::Read
    } else if !message.delivered_to.is_empty() {
        MessageStatus::Delivered
    } else if message.status == SendStatus::Sent {
        MessageStatus::Sent
    } else {
        MessageStatus::Queued
    };
    MessageView {
        id: URL_SAFE_NO_PAD.encode(message.id.0),
        seq: entry.seq,
        outgoing: message.outgoing,
        text,
        deleted: message.deleted,
        unsupported,
        edited: message.edited_at_ms.is_some(),
        sent_at_ms: i64::try_from(message.sent_at_ms).unwrap_or(i64::MAX),
        status,
    }
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
    fn two_sessions_meet_and_talk_through_the_app_api() {
        let dir = tempfile::tempdir().expect("dir");
        let mut stub = Stub::default();
        let mut alice = person(&mut stub, &dir, "alice");
        let mut bob = person(&mut stub, &dir, "bob");

        let (status, body) = stub.call(&lookup_call("Bob".into()).expect("lookup"));
        let chat = alice.start_chat("bob".into(), status, body).expect("chat");
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
        alice.delete_chat(chat).expect("delete");
        assert!(alice.chats().expect("chats").is_empty());
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
