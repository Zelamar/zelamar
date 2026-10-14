//! The whole client side of the delivery service, end to end: two people
//! claim usernames, register devices, find each other, and talk. The service
//! is a small in-memory stand-in that routes like the real one
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
use zelamar_core::content::{AppMessage, Body, MessageId};
use zelamar_core::device::{Author, ChatId, Device, Outgoing, Received};
use zelamar_core::envelope::{Envelope, EnvelopeId, MailboxId};
use zelamar_core::history::SendStatus;
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
                    return answer(404, &json!({ "error": "unknown_account" }));
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
                answer(200, &json!({ "account": key, "devices": listed }))
            }
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

        let claim = service::claim(&account, name, "beta-invite", NOW, WORK_BITS).expect("claim");
        let (status, body) = service.handle(&claim);
        assert_eq!(
            service::claimed(status, &body).expect("claimed"),
            name.to_ascii_lowercase()
        );

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

#[test]
fn two_people_find_each_other_by_username_and_talk() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "Bob_Smith");

    let (status, body) = service.handle(&service::lookup("bob_smith").expect("lookup"));
    let bob_found = service::contact(status, &body).expect("contact");
    assert_eq!(bob_found.account, bob.account.id());
    let (chat, _) = alice
        .device
        .start_chat(bob_found.account, &bob_found.key_packages)
        .expect("start chat");
    alice
        .device
        .send(&chat, &text(1, "Hi Bob, it's Alice"))
        .expect("send");
    alice.flush(&mut service);
    let sent = &alice.device.history(&chat, None, 1).expect("history")[0].message;
    assert_eq!(sent.status, SendStatus::Sent);

    let inbox = bob.read(&mut service);
    assert_eq!(
        inbox[0],
        Received::Joined {
            chat: chat.clone(),
            by: alice.author(),
        }
    );
    assert!(
        matches!(&inbox[1], Received::Message { author, message, .. }
            if *author == alice.author() && message.body == Body::Text { text: "Hi Bob, it's Alice".into() })
    );

    bob.device
        .send(&chat, &text(2, "Hi Alice!"))
        .expect("reply");
    bob.flush(&mut service);
    let replies = alice.read(&mut service);
    assert!(matches!(&replies[..], [Received::Message { author, .. }] if *author == bob.author()));
}

#[test]
fn the_directory_cannot_slip_in_someone_elses_device() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let bob = Person::join(&mut service, "bob");
    let mut mallory = Person::join(&mut service, "mallory");

    // A dishonest directory lists Bob's genuine certificate with a key
    // package from Mallory's device.
    let (status, body) = service.handle(&service::lookup("bob").expect("lookup"));
    let mut found = service::contact(status, &body).expect("contact");
    found.key_packages = vec![mallory.device.key_package().expect("key package")];

    assert!(matches!(
        alice
            .device
            .start_chat(bob.account.id(), &found.key_packages),
        Err(CoreError::Unauthenticated(_))
    ));
    assert_eq!(
        alice.device.chats(),
        Vec::<ChatId>::new(),
        "nothing was created"
    );
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

    // And unknown usernames come back as the service's own answer.
    let (status, body) = service.handle(&service::lookup("nobody").expect("lookup"));
    assert!(matches!(
        service::contact(status, &body),
        Err(CoreError::Service { status: 404, code }) if code == "unknown_account"
    ));
    assert!(service::lookup("no").is_err());
    assert_eq!(b64(b"x"), "eA");
}
