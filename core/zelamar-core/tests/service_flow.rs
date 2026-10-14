//! The whole client side of the delivery service, end to end: people claim
//! usernames, register devices, find each other, and talk. The service is a
//! small in-memory stand-in that routes like the real one
//! (`protocol/delivery.md`, `protocol/directory.md`); the real one checks the
//! same bytes through the shared vectors.

// An integration-test crate: helpers outside `#[test]` functions may fail
// loudly too, because a failed expectation is exactly what a test reports.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, VecDeque};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use zelamar_core::CoreError;
use zelamar_core::contacts::Contact;
use zelamar_core::content::{AppMessage, Body, MessageId};
use zelamar_core::device::{Author, ChatId, Device, Outgoing, Received};
use zelamar_core::envelope::{Envelope, EnvelopeId, MailboxId};
use zelamar_core::history::{SendStatus, StoredContent};
use zelamar_core::identity::AccountKey;
use zelamar_core::service::{self, KEY_PACKAGES_AT_REGISTRATION, Request};
use zelamar_core::store::MasterKey;

const NOW: u64 = 1_760_000_000_000;
/// Easy work for tests; the real service asks for `CLAIM_WORK_BITS`.
const WORK_BITS: u8 = 4;

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(text).expect("base64url")
}

/// A registered device in the stand-in directory.
struct Listed {
    certificate: String,
    key_packages: VecDeque<String>,
    last_resort: String,
}

/// An in-memory delivery service: routes, stores and hands out, nothing more.
#[derive(Default)]
struct Service {
    accounts: HashMap<String, (String, Vec<Listed>)>,
    mailboxes: HashMap<[u8; 32], VecDeque<Vec<u8>>>,
}

impl Service {
    fn handle(&mut self, request: &Request) -> (u16, Vec<u8>) {
        let parts: Vec<&str> = request.path.split('/').skip(2).collect();
        let json_body = || serde_json::from_slice::<Value>(&request.body).expect("json body");
        let answer = |status: u16, body: &Value| (status, serde_json::to_vec(body).expect("json"));
        let unknown = || answer(404, &json!({ "error": "unknown_account" }));
        match (request.method, parts.as_slice()) {
            ("POST", ["accounts"]) => {
                let body = json_body();
                let name = body["username"].as_str().expect("username").to_owned();
                let key = body["account"].as_str().expect("account").to_owned();
                self.accounts.insert(name.clone(), (key, Vec::new()));
                answer(201, &json!({ "username": name }))
            }
            ("PUT", ["accounts", name, "devices", _]) => {
                let body = json_body();
                let certificate = request
                    .headers
                    .iter()
                    .find(|(header, _)| *header == "zelamar-certificate")
                    .expect("certificate header")
                    .1
                    .clone();
                let mailbox: [u8; 32] = unb64(body["mailbox"].as_str().expect("mailbox"))
                    .try_into()
                    .expect("32 bytes");
                self.mailboxes.entry(mailbox).or_default();
                let key_packages: VecDeque<String> = body["key_packages"]
                    .as_array()
                    .expect("key packages")
                    .iter()
                    .map(|kp| kp.as_str().expect("string").to_owned())
                    .collect();
                let count = key_packages.len();
                let devices = &mut self.accounts.get_mut(*name).expect("account").1;
                devices.push(Listed {
                    certificate,
                    key_packages,
                    last_resort: body["last_resort"]
                        .as_str()
                        .expect("last resort")
                        .to_owned(),
                });
                answer(200, &json!({ "key_packages": count }))
            }
            ("GET", ["accounts", name]) => {
                let Some((key, devices)) = self.accounts.get_mut(*name) else {
                    return unknown();
                };
                let listed: Vec<Value> = devices
                    .iter_mut()
                    .map(|device| {
                        let key_package = device
                            .key_packages
                            .pop_front()
                            .unwrap_or_else(|| device.last_resort.clone());
                        json!({ "certificate": device.certificate, "key_package": key_package })
                    })
                    .collect();
                answer(
                    200,
                    &json!({ "username": name, "account": key, "devices": listed }),
                )
            }
            ("GET", ["accounts", name, "key"]) => match self.accounts.get(*name) {
                Some((key, _)) => answer(200, &json!({ "username": name, "account": key })),
                None => unknown(),
            },
            ("POST", ["deliver"]) => {
                let frame = &request.body;
                let id = EnvelopeId(frame[3..19].try_into().expect("id"));
                let count = usize::from(frame[19]);
                let payload = frame[20 + 32 * count + 4..].to_vec();
                let mut delivered = 0;
                for at in 0..count {
                    let start = 20 + 32 * at;
                    let to: [u8; 32] = frame[start..start + 32].try_into().expect("mailbox");
                    if let Some(queue) = self.mailboxes.get_mut(&to) {
                        let envelope = Envelope {
                            id,
                            to: MailboxId(to),
                            payload: payload.clone(),
                        };
                        queue.push_back(envelope.encode().expect("envelope"));
                        delivered += 1;
                    }
                }
                answer(202, &json!({ "delivered": delivered }))
            }
            other => panic!("the stand-in service does not route {other:?}"),
        }
    }

    /// Everything waiting in the mailbox the device reads.
    fn take(&mut self, device: &Device) -> Vec<Vec<u8>> {
        let path = device.mailbox_access().path;
        let id: [u8; 32] = unb64(path.rsplit('/').next().expect("mailbox id"))
            .try_into()
            .expect("32 bytes");
        self.mailboxes
            .get_mut(&id)
            .expect("registered mailbox")
            .drain(..)
            .collect()
    }
}

struct Person {
    account: AccountKey,
    device: Device,
    _dir: tempfile::TempDir,
}

impl Person {
    /// Claims `name` and registers a first device.
    fn join(service: &mut Service, name: &str) -> Self {
        let account = AccountKey::generate().expect("account");
        let dir = tempfile::tempdir().expect("dir");
        let mut device = Device::create(
            &dir.path().join("device.db"),
            &MasterKey::generate().expect("master key"),
            &account,
        )
        .expect("device");

        let claim = device
            .claim(name, "beta-invite", NOW, WORK_BITS)
            .expect("claim");
        let (status, body) = service.handle(&claim);
        let normalized = name.to_ascii_lowercase();
        assert_eq!(device.claimed(status, &body).expect("claimed"), normalized);
        assert_eq!(device.username(), Some(normalized.as_str()));

        let registration = device.register(name, NOW).expect("registration");
        let (status, body) = service.handle(&registration);
        assert_eq!(
            service::key_package_count(status, &body).expect("registered"),
            KEY_PACKAGES_AT_REGISTRATION as u64
        );
        Self {
            account,
            device,
            _dir: dir,
        }
    }

    const fn author(&self) -> Author {
        Author {
            account: self.account.id(),
            device: self.device.id(),
        }
    }

    /// Looks `name` up and starts a chat with every device found.
    fn start_chat_with(&mut self, service: &mut Service, name: &str) -> ChatId {
        let (status, body) = service.handle(&service::lookup(name).expect("lookup"));
        let entry = service::directory_entry(name, status, &body).expect("entry");
        self.device.start_chat(&entry).expect("start chat").0
    }

    /// Sends everything in the outbox and confirms what the service took.
    fn flush(&mut self, service: &mut Service) {
        for outgoing in self.device.outbox().expect("outbox") {
            let (status, body) = service.handle(&service::delivery(&outgoing).expect("frame"));
            service::delivered(status, &body).expect("delivered");
            self.device.confirm_sent(outgoing.id).expect("confirm");
        }
        assert_eq!(
            self.device.outbox().expect("outbox"),
            Vec::<Outgoing>::new()
        );
    }

    /// Opens everything in the mailbox; every envelope is acknowledged.
    fn read(&mut self, service: &mut Service) -> Vec<Received> {
        service
            .take(&self.device)
            .iter()
            .map(|bytes| {
                let opened = self.device.open_envelope(bytes).expect("an envelope");
                assert!(!opened.keep(), "nothing transient happens in memory");
                opened.result.expect("received")
            })
            .collect()
    }
}

fn text(n: u8, body: &str) -> AppMessage {
    AppMessage::text(MessageId([n; 16]), NOW + u64::from(n), body)
}

fn is_profile(received: &Received, name: &str) -> bool {
    matches!(received, Received::Message { message, .. }
        if message.body == Body::Profile { username: name.into() })
}

fn is_text(received: &Received, from: Author, expected: &str) -> bool {
    matches!(received, Received::Message { author, message, .. }
        if *author == from && message.body == Body::Text { text: expected.into() })
}

#[test]
fn two_people_find_each_other_by_username_and_talk() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "Bob_Smith");

    let chat = alice.start_chat_with(&mut service, "bob_smith");
    alice
        .device
        .send(&chat, &text(1, "Hi Bob, it's Alice"))
        .expect("send");
    alice.flush(&mut service);
    let sent = &alice.device.history(&chat, None, 1).expect("history")[0].message;
    assert_eq!(sent.status, SendStatus::Sent);

    // The welcome, then Alice's profile (once per chat), then her message.
    let inbox = bob.read(&mut service);
    assert_eq!(inbox.len(), 3);
    assert_eq!(
        inbox[0],
        Received::Joined {
            chat: chat.clone(),
            by: alice.author(),
        }
    );
    assert!(is_profile(&inbox[1], "alice"));
    assert!(is_text(&inbox[2], alice.author(), "Hi Bob, it's Alice"));

    for (n, line) in [(2, "Hi Alice!"), (3, "Dinner at eight?")] {
        bob.device.send(&chat, &text(n, line)).expect("reply");
    }
    bob.flush(&mut service);
    let replies = alice.read(&mut service);
    assert_eq!(replies.len(), 3, "Bob introduces himself once");
    assert!(is_profile(&replies[0], "bob_smith"));
    assert!(is_text(&replies[1], bob.author(), "Hi Alice!"));
    assert!(is_text(&replies[2], bob.author(), "Dinner at eight?"));
}

#[test]
fn usernames_are_claimed_in_chat_and_confirmed_by_the_directory() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");

    let chat = alice.start_chat_with(&mut service, "bob");
    alice.device.send(&chat, &text(1, "Dinner?")).expect("send");
    alice.flush(&mut service);
    bob.read(&mut service);

    // Alice looked Bob up, so she knows his name for certain. Bob only has
    // Alice's word for hers.
    assert_eq!(
        alice.device.contacts(),
        vec![Contact {
            account: bob.account.id(),
            username: Some("bob".into()),
            verified: true,
        }]
    );
    let unconfirmed = bob.device.unconfirmed_contacts();
    assert_eq!(
        unconfirmed,
        vec![Contact {
            account: alice.account.id(),
            username: Some("alice".into()),
            verified: false,
        }]
    );

    let (status, body) = service.handle(&service::key_lookup("alice").expect("key lookup"));
    let answer = service::directory_entry("alice", status, &body).expect("entry");
    assert_eq!(
        answer.key_packages,
        Vec::<Vec<u8>>::new(),
        "takes no key package"
    );
    assert!(bob.device.confirm_contact(&answer).expect("confirm"));
    assert_eq!(bob.device.unconfirmed_contacts(), Vec::new());

    // A directory answer about someone else confirms nothing.
    let (status, body) = service.handle(&service::key_lookup("bob").expect("key lookup"));
    let other = service::directory_entry("bob", status, &body).expect("entry");
    assert!(!bob.device.confirm_contact(&other).expect("confirm"));

    let chats = bob.device.chat_list().expect("chat list");
    assert_eq!(chats.len(), 1);
    assert_eq!(
        chats[0].members,
        vec![Contact {
            account: alice.account.id(),
            username: Some("alice".into()),
            verified: true,
        }]
    );
    let newest = chats[0].last.as_ref().expect("a last message");
    assert_eq!(
        newest.message.content,
        StoredContent::Text {
            text: "Dinner?".into()
        }
    );
}

#[test]
fn the_directory_cannot_slip_in_someone_elses_device() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let bob = Person::join(&mut service, "bob");
    let mut mallory = Person::join(&mut service, "mallory");

    // A dishonest directory lists Bob's account with a key package from
    // Mallory's device.
    let (status, body) = service.handle(&service::lookup("bob").expect("lookup"));
    let mut found = service::directory_entry("bob", status, &body).expect("entry");
    assert_eq!(found.account, bob.account.id());
    found.key_packages = vec![mallory.device.key_package().expect("key package")];

    assert!(matches!(
        alice.device.start_chat(&found),
        Err(CoreError::Unauthenticated(_))
    ));
    assert_eq!(
        alice.device.chats(),
        Vec::<ChatId>::new(),
        "nothing was created"
    );
    assert_eq!(alice.device.contacts(), Vec::new(), "nothing was learned");
    assert_eq!(
        alice.device.outbox().expect("outbox"),
        Vec::<Outgoing>::new()
    );
}

#[test]
fn an_envelope_for_another_mailbox_is_refused_but_acknowledged() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let stray = Envelope {
        id: EnvelopeId([9; 16]),
        to: MailboxId([1; 32]),
        payload: vec![0; 8],
    }
    .encode()
    .expect("envelope");
    let opened = alice.device.open_envelope(&stray).expect("an envelope");
    assert_eq!(opened.id, EnvelopeId([9; 16]));
    assert!(matches!(opened.result, Err(CoreError::Malformed { .. })));
    assert!(!opened.keep());
    assert_eq!(service::acknowledgement(&[opened.id])[0], 0x01);

    // Unknown usernames come back as the service's own answer.
    let (status, body) = service.handle(&service::lookup("nobody").expect("lookup"));
    assert!(matches!(
        service::directory_entry("nobody", status, &body),
        Err(CoreError::Service { status: 404, code }) if code == "unknown_account"
    ));
    assert!(service::lookup("no").is_err());
    assert_eq!(b64(b"x"), "eA");
}
