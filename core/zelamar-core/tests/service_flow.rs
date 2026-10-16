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
use zelamar_core::device::{Author, ChatId, Device, Outgoing, Received, Settled};
use zelamar_core::envelope::{Envelope, EnvelopeId, MailboxId};
use zelamar_core::history::{SendStatus, StoredContent};
use zelamar_core::identity::{AccountId, AccountKey};
use zelamar_core::link::{self, Joining, Offer};
use zelamar_core::service::{self, DirectoryEntry, KEY_PACKAGES_AT_REGISTRATION, Request};
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
    id: String,
    certificate: String,
    key_packages: VecDeque<String>,
    last_resort: String,
}

/// An in-memory delivery service: routes, stores and hands out, nothing more.
#[derive(Default)]
struct Service {
    accounts: HashMap<String, (String, Vec<Listed>)>,
    mailboxes: HashMap<[u8; 32], VecDeque<Vec<u8>>>,
    /// The sequencer: per chat, the current epoch and the last winner.
    groups: HashMap<String, (u64, [u8; 16])>,
    /// Link slots: the sealed request and, once approved, the answer.
    links: HashMap<String, (Vec<u8>, Option<Vec<u8>>)>,
}

/// One delivery frame read from the start of `frame`, and its length.
fn read_frame(frame: &[u8]) -> (EnvelopeId, Vec<[u8; 32]>, Vec<u8>, usize) {
    let id = EnvelopeId(frame[3..19].try_into().expect("id"));
    let count = usize::from(frame[19]);
    let recipients = (0..count)
        .map(|at| {
            frame[20 + 32 * at..52 + 32 * at]
                .try_into()
                .expect("mailbox")
        })
        .collect();
    let length_at = 20 + 32 * count;
    let length = u32::from_be_bytes(frame[length_at..length_at + 4].try_into().expect("u32"));
    let end = length_at + 4 + length as usize;
    (id, recipients, frame[length_at + 4..end].to_vec(), end)
}

impl Service {
    #[expect(
        clippy::too_many_lines,
        reason = "one match is the routing table, like the service's own"
    )]
    fn handle(&mut self, request: &Request) -> (u16, Vec<u8>) {
        let parts: Vec<&str> = request.path.split('/').skip(2).collect();
        let json_body = || serde_json::from_slice::<Value>(&request.body).expect("json body");
        let answer = |status: u16, body: &Value| (status, serde_json::to_vec(body).expect("json"));
        let unknown = || answer(404, &json!({ "error": "unknown_account" }));
        if let ["links", slot @ ..] = parts.as_slice() {
            return self.link_slot(request, slot);
        }
        match (request.method, parts.as_slice()) {
            ("POST", ["accounts"]) => {
                let body = json_body();
                let name = body["username"].as_str().expect("username").to_owned();
                let key = body["account"].as_str().expect("account").to_owned();
                self.accounts.insert(name.clone(), (key, Vec::new()));
                answer(201, &json!({ "username": name }))
            }
            ("PUT", ["accounts", name, "devices", id]) => {
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
                    id: (*id).to_owned(),
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
            ("GET", ["accounts", name, "devices"]) => {
                let Some((key, devices)) = self.accounts.get(*name) else {
                    return unknown();
                };
                let listed: Vec<Value> = devices
                    .iter()
                    .map(|device| json!({ "certificate": device.certificate }))
                    .collect();
                answer(
                    200,
                    &json!({ "username": name, "account": key, "devices": listed }),
                )
            }
            ("DELETE", ["accounts", name, "devices", id]) => {
                let Some((_, devices)) = self.accounts.get_mut(*name) else {
                    return unknown();
                };
                devices.retain(|device| device.id != *id);
                answer(200, &json!({ "devices": devices.len() }))
            }
            ("GET", ["accounts", name, "devices", id]) => {
                let Some((key, devices)) = self.accounts.get_mut(*name) else {
                    return unknown();
                };
                let Some(device) = devices.iter_mut().find(|device| device.id == *id) else {
                    return answer(404, &json!({ "error": "unknown_device" }));
                };
                let key_package = device
                    .key_packages
                    .pop_front()
                    .unwrap_or_else(|| device.last_resort.clone());
                let listed =
                    json!({ "certificate": device.certificate, "key_package": key_package });
                answer(
                    200,
                    &json!({ "username": name, "account": key, "devices": [listed] }),
                )
            }
            ("GET", ["accounts", name, "key"]) => match self.accounts.get(*name) {
                Some((key, _)) => answer(200, &json!({ "username": name, "account": key })),
                None => unknown(),
            },
            ("POST", ["deliver"]) => {
                let (id, recipients, payload, _) = read_frame(&request.body);
                let delivered = self.fan_out(id, &recipients, &payload);
                answer(202, &json!({ "delivered": delivered }))
            }
            ("POST", ["groups", group, "commits"]) => {
                let body = &request.body;
                let epoch = u64::from_be_bytes(body[3..11].try_into().expect("epoch"));
                let (id, recipients, payload, used) = read_frame(&body[11..]);
                let welcome = (11 + used < body.len()).then(|| read_frame(&body[11 + used..]));
                let state = self.groups.get(*group).copied();
                match state {
                    Some((current, last)) if current == epoch + 1 && last == id.0 => {}
                    Some((current, _)) if current != epoch => {
                        return answer(
                            409,
                            &json!({ "error": "epoch_mismatch", "epoch": current }),
                        );
                    }
                    _ => {
                        self.groups.insert((*group).to_owned(), (epoch + 1, id.0));
                    }
                }
                let delivered = self.fan_out(id, &recipients, &payload);
                let welcomed =
                    welcome.map_or(0, |(id, to, payload, _)| self.fan_out(id, &to, &payload));
                answer(
                    202,
                    &json!({ "epoch": epoch + 1, "delivered": delivered, "welcomed": welcomed }),
                )
            }
            other => panic!("the stand-in service does not route {other:?}"),
        }
    }

    /// Link slots: the sealed request, then the answer, first come first
    /// served (`protocol/linking.md`).
    fn link_slot(&mut self, request: &Request, parts: &[&str]) -> (u16, Vec<u8>) {
        let answer = |status: u16, body: &Value| (status, serde_json::to_vec(body).expect("json"));
        match (request.method, parts) {
            ("PUT", [slot, "request"]) => match self.links.get(*slot) {
                Some((stored, _)) if *stored != request.body => {
                    answer(409, &json!({ "error": "link_taken" }))
                }
                Some(_) => answer(200, &json!({})),
                None => {
                    self.links
                        .insert((*slot).to_owned(), (request.body.clone(), None));
                    answer(201, &json!({}))
                }
            },
            ("PUT", [slot, "answer"]) => match self.links.get_mut(*slot) {
                None => answer(404, &json!({ "error": "unknown_link" })),
                Some((_, Some(_))) => answer(409, &json!({ "error": "link_taken" })),
                Some((_, answered)) => {
                    *answered = Some(request.body.clone());
                    answer(201, &json!({}))
                }
            },
            ("GET", [slot]) => match self.links.get(*slot) {
                None => answer(404, &json!({ "error": "unknown_link" })),
                Some((request, answered)) => answer(
                    200,
                    &json!({ "request": b64(request), "answer": answered.as_deref().map(b64) }),
                ),
            },
            ("DELETE", [slot]) => {
                self.links.remove(*slot);
                (204, Vec::new())
            }
            other => panic!("the stand-in service does not route {other:?}"),
        }
    }

    fn fan_out(&mut self, id: EnvelopeId, recipients: &[[u8; 32]], payload: &[u8]) -> usize {
        let mut delivered = 0;
        for to in recipients {
            if let Some(queue) = self.mailboxes.get_mut(to) {
                let envelope = Envelope {
                    id,
                    to: MailboxId(*to),
                    payload: payload.to_vec(),
                };
                queue.push_back(envelope.encode().expect("envelope"));
                delivered += 1;
            }
        }
        delivered
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
    account: AccountId,
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
            account: account.id(),
            device,
            _dir: dir,
        }
    }

    const fn author(&self) -> Author {
        Author {
            account: self.account,
            device: self.device.id(),
        }
    }

    /// Looks `name` up and starts a chat with every device found, and with
    /// this account's other devices.
    fn start_chat_with(&mut self, service: &mut Service, name: &str) -> ChatId {
        let (status, body) = service.handle(&service::lookup(name).expect("lookup"));
        let entry = service::directory_entry(name, status, &body).expect("entry");
        let own = self.device.username().expect("username").to_owned();
        let (status, body) = service.handle(&service::lookup(&own).expect("lookup"));
        let siblings = service::directory_entry(&own, status, &body).expect("own entry");
        self.device
            .start_chat(&entry, &[siblings])
            .expect("start chat")
            .0
    }

    /// Links a new device to this account, the way the two apps do.
    fn link(&mut self, service: &mut Service, label: &str) -> Self {
        let offer = Offer::new().expect("offer");
        let (status, body) = service.handle(&offer.check());
        assert_eq!(offer.applicant(status, &body).expect("slot"), None);

        let joining = Joining::new(&offer.code().to_uppercase(), label).expect("joining");
        let (status, body) = service.handle(&joining.request().expect("request"));
        link::stored(status, &body).expect("request stored");
        let (status, body) = service.handle(&joining.check());
        assert!(joining.linked(status, &body).expect("waiting").is_none());

        let (status, body) = service.handle(&offer.check());
        let applicant = offer
            .applicant(status, &body)
            .expect("applicant")
            .expect("someone asked");
        assert_eq!(applicant.label, label);
        assert_eq!(applicant.match_code, joining.match_code());
        let approval = self
            .device
            .approve_link(&offer, &applicant)
            .expect("approve");
        let (status, body) = service.handle(&approval);
        link::stored(status, &body).expect("answer stored");

        let (status, body) = service.handle(&joining.check());
        let linked = joining
            .linked(status, &body)
            .expect("answer")
            .expect("approved");
        service.handle(&joining.close());
        let dir = tempfile::tempdir().expect("dir");
        let mut device = Device::create_linked(
            &dir.path().join("device.db"),
            &MasterKey::generate().expect("master key"),
            joining,
            linked,
        )
        .expect("linked device");
        let name = device.username().expect("username").to_owned();
        let (status, body) = service.handle(&device.register(&name, NOW).expect("register"));
        service::key_package_count(status, &body).expect("registered");
        Self {
            account: device.account(),
            device,
            _dir: dir,
        }
    }

    /// Takes removed devices out of every chat and brings this account's
    /// other devices into every chat that lacks them; returns how many
    /// chats changed.
    fn heal(&mut self, service: &mut Service) -> usize {
        let name = self.device.username().expect("username").to_owned();
        let (status, body) = service.handle(&service::devices(&name).expect("devices"));
        let (account, registered) =
            service::device_list(&name, status, &body).expect("device list");
        assert_eq!(account, self.account);
        let stale = self.device.stale_devices(&registered).expect("stale");
        for (chat, devices) in &stale {
            self.device
                .remove_own_devices(chat, devices)
                .expect("remove devices");
        }
        let missing = self.device.missing_devices(&registered).expect("missing");
        for (chat, devices) in &missing {
            let entries: Vec<_> = devices
                .iter()
                .map(|device| {
                    let lookup = service::device_lookup(&name, *device).expect("lookup");
                    let (status, body) = service.handle(&lookup);
                    service::directory_entry(&name, status, &body).expect("entry")
                })
                .collect();
            self.device
                .add_own_devices(chat, &entries)
                .expect("add devices");
        }
        stale.len() + missing.len()
    }

    /// Sends everything in the outbox and confirms what the service took.
    /// Sends everything in the outbox and settles each answer. Settling may
    /// queue more (a name for people just added): this sends until the
    /// outbox is empty.
    fn flush(&mut self, service: &mut Service) -> Vec<Settled> {
        let mut settled = Vec::new();
        loop {
            let outbox = self.device.outbox().expect("outbox");
            if outbox.is_empty() {
                return settled;
            }
            for outgoing in outbox {
                let (status, body) =
                    service.handle(&service::delivery(&outgoing).expect("request"));
                settled.push(
                    self.device
                        .settle(outgoing.id, status, &body)
                        .expect("settle"),
                );
            }
        }
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
            account: bob.account,
            username: Some("bob".into()),
            verified: true,
        }]
    );
    let unconfirmed = bob.device.unconfirmed_contacts();
    assert_eq!(
        unconfirmed,
        vec![Contact {
            account: alice.account,
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
            account: alice.account,
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
    assert_eq!(found.account, bob.account);
    found.key_packages = vec![mallory.device.key_package().expect("key package")];

    assert!(matches!(
        alice.device.start_chat(&found, &[]),
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

#[test]
fn receipts_flow_only_once_the_user_has_written_in_the_chat() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");
    let status = |person: &Person, chat: &ChatId| {
        person.device.history(chat, None, 1).expect("history")[0]
            .message
            .clone()
    };

    let chat = alice.start_chat_with(&mut service, "bob");
    alice.device.send(&chat, &text(1, "Hi Bob")).expect("send");
    alice.flush(&mut service);
    bob.read(&mut service);

    // Bob has not answered: reading tells Alice nothing.
    assert_eq!(bob.device.chat_list().expect("list")[0].unread, 1);
    assert_eq!(bob.device.mark_read(&chat).expect("read"), 1);
    assert_eq!(bob.device.chat_list().expect("list")[0].unread, 0);
    bob.flush(&mut service);
    assert!(
        alice.read(&mut service).is_empty(),
        "no receipt from a stranger"
    );
    assert_eq!(status(&alice, &chat).delivered_to, Vec::new());

    // Once Bob writes back, receipts flow both ways.
    bob.device.send(&chat, &text(2, "Hi Alice")).expect("reply");
    bob.flush(&mut service);
    alice.read(&mut service);
    alice.flush(&mut service);
    bob.read(&mut service);
    assert_eq!(status(&bob, &chat).delivered_to, vec![alice.account]);

    assert_eq!(alice.device.mark_read(&chat).expect("read"), 1);
    alice.flush(&mut service);
    bob.read(&mut service);
    assert_eq!(status(&bob, &chat).read_by, vec![alice.account]);

    alice.device.send(&chat, &text(3, "Dinner?")).expect("send");
    alice.flush(&mut service);
    bob.read(&mut service);
    bob.flush(&mut service);
    alice.read(&mut service);
    assert_eq!(status(&alice, &chat).delivered_to, vec![bob.account]);
}

#[test]
fn a_chat_request_can_be_declined_without_a_trace_or_accepted() {
    let mut service = Service::default();
    let mut bob = Person::join(&mut service, "bob");
    let mut mallory = Person::join(&mut service, "mallory");
    let mut alice = Person::join(&mut service, "alice");

    // A stranger writes: Bob sees a request.
    let spam = mallory.start_chat_with(&mut service, "bob");
    mallory
        .device
        .send(&spam, &text(1, "Buy now"))
        .expect("send");
    mallory.flush(&mut service);
    bob.read(&mut service);
    let list = bob.device.chat_list().expect("list");
    assert!(list[0].request);

    // Declined: gone, history and all; what follows is refused and dropped.
    bob.device.delete_chat(&spam).expect("decline");
    assert_eq!(bob.device.chat_list().expect("list").len(), 0);
    assert!(matches!(
        bob.device.history(&spam, None, 10),
        Err(CoreError::UnknownChat)
    ));
    mallory
        .device
        .send(&spam, &text(2, "Hello?"))
        .expect("send");
    mallory.flush(&mut service);
    for bytes in service.take(&bob.device) {
        let opened = bob.device.open_envelope(&bytes).expect("an envelope");
        assert!(matches!(opened.result, Err(CoreError::UnknownChat)));
        assert!(!opened.keep(), "acknowledged, so it never comes back");
    }
    assert!(
        mallory.read(&mut service).is_empty(),
        "Mallory learns nothing"
    );

    // Accepted: Bob introduces himself and the chat is an ordinary one.
    let chat = alice.start_chat_with(&mut service, "bob");
    alice.device.send(&chat, &text(3, "Hi Bob")).expect("send");
    alice.flush(&mut service);
    bob.read(&mut service);
    assert!(bob.device.chat_list().expect("list")[0].request);
    bob.device.accept_chat(&chat).expect("accept");
    assert!(!bob.device.chat_list().expect("list")[0].request);
    bob.flush(&mut service);
    let introduced = alice.read(&mut service);
    assert!(
        matches!(&introduced[..], [Received::Message { message, .. }]
        if message.body == Body::Profile { username: "bob".into() })
    );
}

#[test]
fn rival_changes_to_a_chat_are_sequenced_and_nobody_forks() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");
    let mut carol = Person::join(&mut service, "carol");
    let mut dave = Person::join(&mut service, "dave");

    let chat = alice.start_chat_with(&mut service, "bob");
    alice.flush(&mut service);
    bob.read(&mut service);

    let invite = |person: &mut Person, service: &mut Service, name: &str| {
        let (status, body) = service.handle(&service::lookup(name).expect("lookup"));
        let entry = service::directory_entry(name, status, &body).expect("entry");
        person
            .device
            .invite(&chat, &entry.key_packages[0])
            .expect("invite")
    };

    // Both change the chat in the same epoch.
    let carols = invite(&mut alice, &mut service, "carol");
    let daves = invite(&mut bob, &mut service, "dave");
    assert!(carols.commit.commit.is_some() && daves.commit.commit.is_some());
    // A message written while Alice's commit waits goes out in the old epoch.
    alice
        .device
        .send(&chat, &text(1, "while waiting"))
        .expect("send");

    // Alice reaches the sequencer first; Bob is refused.
    assert_eq!(
        alice.flush(&mut service),
        vec![Settled::Applied, Settled::Delivered]
    );
    assert_eq!(bob.flush(&mut service), vec![Settled::Refused]);
    assert!(
        dave.read(&mut service).is_empty(),
        "Dave was never welcomed"
    );

    // Bob applies the winner and still reads the message from the old epoch.
    let bobs = bob.read(&mut service);
    assert!(matches!(bobs[0], Received::ChatUpdated { .. }));
    assert!(is_text(&bobs[1], alice.author(), "while waiting"));
    assert!(matches!(
        carol.read(&mut service)[0],
        Received::Joined { .. }
    ));

    // Bob tries again, on top: now it wins, and everyone follows.
    invite(&mut bob, &mut service, "dave");
    assert_eq!(bob.flush(&mut service), vec![Settled::Applied]);
    for person in [&mut alice, &mut carol] {
        assert!(matches!(
            person.read(&mut service)[0],
            Received::ChatUpdated { .. }
        ));
    }
    assert!(matches!(
        dave.read(&mut service)[0],
        Received::Joined { .. }
    ));

    // One chat, four members, one epoch: a message reaches all of them.
    alice
        .device
        .send(&chat, &text(2, "Welcome, both of you"))
        .expect("send");
    alice.flush(&mut service);
    for person in [&mut bob, &mut carol, &mut dave] {
        let got = person.read(&mut service);
        assert!(
            got.iter()
                .any(|r| is_text(r, alice.author(), "Welcome, both of you")),
            "{got:?}"
        );
    }
}

fn texts(received: &[Received]) -> Vec<(Author, String)> {
    received
        .iter()
        .filter_map(|r| match r {
            Received::Message {
                author,
                message:
                    AppMessage {
                        body: Body::Text { text },
                        ..
                    },
                ..
            } => Some((*author, text.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_linked_device_joins_the_account_and_every_chat() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");
    let chat = alice.start_chat_with(&mut service, "bob");
    alice
        .device
        .send(&chat, &text(1, "before the phone"))
        .expect("send");
    alice.flush(&mut service);
    bob.read(&mut service);

    let mut phone = alice.link(&mut service, "Android phone");
    assert_eq!(phone.device.username(), Some("alice"));
    assert_eq!(phone.device.account(), alice.device.account());
    assert!(!phone.device.holds_account_key());
    assert!(
        phone
            .device
            .contacts()
            .iter()
            .any(|c| c.username.as_deref() == Some("bob") && c.verified),
        "contacts come along"
    );

    // Alice's first device brings the phone into her chat with Bob.
    assert_eq!(alice.heal(&mut service), 1);
    assert_eq!(alice.flush(&mut service), vec![Settled::Applied]);
    assert!(matches!(
        bob.read(&mut service)[0],
        Received::ChatUpdated { .. }
    ));
    let joined = phone.read(&mut service);
    assert!(
        matches!(&joined[0], Received::Joined { chat: c, by } if *c == chat && *by == alice.author())
    );
    let list = phone.device.chat_list().expect("chats");
    assert_eq!(list.len(), 1);
    assert!(!list[0].request, "Alice already spoke there");
    assert!(
        phone
            .device
            .history(&chat, None, 10)
            .expect("history")
            .is_empty(),
        "history starts at linking"
    );
    assert_eq!(alice.heal(&mut service), 0, "nothing left to heal");
    assert_eq!(phone.heal(&mut service), 0);

    // Bob's words reach both of Alice's devices.
    bob.device.send(&chat, &text(2, "hi both")).expect("send");
    bob.flush(&mut service);
    for device in [&mut alice, &mut phone] {
        assert_eq!(
            texts(&device.read(&mut service)),
            vec![(bob.author(), "hi both".to_owned())]
        );
    }

    // The phone answers as Alice; her first device keeps it as her own.
    phone
        .device
        .send(&chat, &text(3, "from my phone"))
        .expect("send");
    phone.flush(&mut service);
    let got = texts(&bob.read(&mut service));
    assert_eq!(got, vec![(phone.author(), "from my phone".to_owned())]);
    assert_eq!(got[0].0.account, alice.account);
    alice.read(&mut service);
    let newest = &alice.device.history(&chat, None, 1).expect("history")[0].message;
    assert!(newest.outgoing);
    assert_eq!(
        newest.content,
        StoredContent::Text {
            text: "from my phone".into()
        }
    );

    // A chat started on the phone holds Alice's first device from the start.
    let mut carol = Person::join(&mut service, "carol");
    let other = phone.start_chat_with(&mut service, "carol");
    phone
        .device
        .send(&other, &text(4, "hello Carol"))
        .expect("send");
    phone.flush(&mut service);
    let carols = carol.read(&mut service);
    assert!(matches!(carols[0], Received::Joined { .. }));
    assert_eq!(
        texts(&carols),
        vec![(phone.author(), "hello Carol".to_owned())]
    );
    let alices = alice.read(&mut service);
    assert!(matches!(&alices[0], Received::Joined { chat: c, .. } if *c == other));
    assert_eq!(
        texts(&alices),
        vec![(phone.author(), "hello Carol".to_owned())]
    );
    assert!(
        !alice
            .device
            .chat_list()
            .expect("chats")
            .iter()
            .any(|summary| summary.request),
        "both chats are Alice's own on both devices"
    );
}

#[test]
fn linking_needs_the_code_and_the_account_key() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let offer = Offer::new().expect("offer");

    // A mistyped code is caught before anything is sent.
    assert!(Joining::new("abcde-fghij", "Laptop").is_err());
    assert!(Joining::new(&offer.code().replace('-', "1"), "Laptop").is_err());

    // Someone with another code meets in another slot: the offer sees nothing.
    let stranger = Joining::new(&Offer::new().expect("offer").code(), "Laptop").expect("joining");
    let (status, body) = service.handle(&stranger.request().expect("request"));
    link::stored(status, &body).expect("stored");
    let (status, body) = service.handle(&offer.check());
    assert_eq!(offer.applicant(status, &body).expect("slot"), None);

    // A request sealed under another code does not open, even in the right slot.
    let joining = Joining::new(&offer.code(), "Laptop").expect("joining");
    let request = joining.request().expect("request");
    let forged = Request {
        body: stranger.request().expect("request").body,
        ..request.clone()
    };
    let (status, body) = service.handle(&forged);
    link::stored(status, &body).expect("stored");
    let (status, body) = service.handle(&offer.check());
    assert!(matches!(
        offer.applicant(status, &body),
        Err(CoreError::Unauthenticated(_))
    ));
    // ...and the slot is taken: the real device hears so.
    let (status, body) = service.handle(&request);
    assert!(matches!(
        link::stored(status, &body),
        Err(CoreError::Service { code, .. }) if code == "link_taken"
    ));

    // A linked device cannot link others.
    let mut phone = alice.link(&mut service, "Phone");
    let offer = Offer::new().expect("offer");
    let joining = Joining::new(&offer.code(), "Tablet").expect("joining");
    let (status, body) = service.handle(&joining.request().expect("request"));
    link::stored(status, &body).expect("stored");
    let (status, body) = service.handle(&offer.check());
    let applicant = offer
        .applicant(status, &body)
        .expect("applicant")
        .expect("asked");
    assert!(matches!(
        phone.device.approve_link(&offer, &applicant),
        Err(CoreError::Unauthenticated(_))
    ));
    assert_eq!(phone.heal(&mut service), 0);
}

#[test]
fn chat_requests_are_not_healed_until_accepted() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");
    let mut phone = alice.link(&mut service, "Phone");

    // Bob looks Alice up after she linked the phone: both devices get it.
    bob.start_chat_with(&mut service, "alice");
    bob.flush(&mut service);
    assert!(matches!(
        alice.read(&mut service)[0],
        Received::Joined { .. }
    ));
    assert!(matches!(
        phone.read(&mut service)[0],
        Received::Joined { .. }
    ));

    // A chat only Alice's first device is in, still a request: Bob learns
    // nothing about Alice's devices until she accepts.
    let mut carol = Person::join(&mut service, "carol");
    let (status, body) = service.handle(&service::lookup("alice").expect("lookup"));
    let mut entry = service::directory_entry("alice", status, &body).expect("entry");
    entry.key_packages.truncate(1);
    let (lone, _) = carol.device.start_chat(&entry, &[]).expect("start");
    carol.flush(&mut service);
    alice.read(&mut service);
    assert_eq!(alice.heal(&mut service), 0, "requests stay as they are");

    alice.device.accept_chat(&lone).expect("accept");
    alice.flush(&mut service);
    assert_eq!(alice.heal(&mut service), 1);
    alice.flush(&mut service);
    let joined = phone.read(&mut service);
    assert!(
        joined
            .iter()
            .any(|r| matches!(r, Received::Joined { chat: c, .. } if *c == lone))
    );
}

#[test]
fn a_removed_device_leaves_every_chat_and_hears_nothing_more() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");
    let mut phone = alice.link(&mut service, "Phone");
    alice
        .device
        .remember_label(alice.device.id(), "Laptop")
        .expect("label");
    assert_eq!(alice.device.device_label(phone.device.id()), Some("Phone"));
    assert_eq!(phone.device.device_label(phone.device.id()), Some("Phone"));

    let chat = alice.start_chat_with(&mut service, "bob");
    alice.flush(&mut service);
    bob.read(&mut service);
    assert!(matches!(
        phone.read(&mut service)[0],
        Received::Joined { .. }
    ));

    // Only the device holding the account key removes other devices.
    assert!(matches!(
        phone.device.remove_device("alice", alice.device.id(), NOW),
        Err(CoreError::Unauthenticated(_))
    ));
    // The phone is lost: Alice takes it out of the directory...
    let removal = alice
        .device
        .remove_device("alice", phone.device.id(), NOW)
        .expect("removal");
    assert!(
        removal
            .headers
            .iter()
            .any(|(name, _)| *name == "zelamar-account-signature")
    );
    let (status, body) = service.handle(&removal);
    service::device_removed(status, &body).expect("removed");

    // ...and healing takes it out of every chat.
    assert_eq!(alice.heal(&mut service), 1);
    assert_eq!(alice.flush(&mut service), vec![Settled::Applied]);
    assert!(matches!(
        bob.read(&mut service)[0],
        Received::ChatUpdated { .. }
    ));
    assert!(matches!(
        &phone.read(&mut service)[0],
        Received::Left { chat: c } if *c == chat
    ));
    assert_eq!(alice.heal(&mut service), 0, "nothing left to heal");

    // Bob's words no longer reach the phone, and it cannot write there.
    bob.device
        .send(&chat, &text(1, "Is it gone?"))
        .expect("send");
    bob.flush(&mut service);
    assert_eq!(phone.read(&mut service), Vec::<Received>::new());
    assert_eq!(
        texts(&alice.read(&mut service)),
        vec![(bob.author(), "Is it gone?".to_owned())]
    );
    assert!(phone.device.send(&chat, &text(2, "still here")).is_err());
}

#[test]
fn a_linked_device_can_unlink_itself() {
    let mut service = Service::default();
    let mut alice = Person::join(&mut service, "alice");
    let mut bob = Person::join(&mut service, "bob");
    let mut phone = alice.link(&mut service, "Phone");
    phone.start_chat_with(&mut service, "bob");
    phone.flush(&mut service);
    bob.read(&mut service);
    alice.read(&mut service);

    let leaving = phone
        .device
        .remove_device("alice", phone.device.id(), NOW)
        .expect("leaving");
    assert!(
        leaving
            .headers
            .iter()
            .any(|(name, _)| *name == "zelamar-certificate")
    );
    let (status, body) = service.handle(&leaving);
    service::device_removed(status, &body).expect("removed");

    // Alice's first device notices and takes the phone out of the chat.
    assert_eq!(alice.heal(&mut service), 1);
    assert_eq!(alice.flush(&mut service), vec![Settled::Applied]);
    assert!(matches!(
        bob.read(&mut service)[0],
        Received::ChatUpdated { .. }
    ));
    assert!(matches!(
        alice
            .device
            .remove_own_devices(&alice.device.chats()[0], &[alice.device.id()]),
        Err(CoreError::Malformed { .. })
    ));
}

impl Person {
    /// A directory entry for `name`, with one key package per device.
    fn entry(service: &mut Service, name: &str) -> DirectoryEntry {
        let (status, body) = service.handle(&service::lookup(name).expect("lookup"));
        service::directory_entry(name, status, &body).expect("entry")
    }

    /// Takes the devices of people who left out of chats, where this
    /// device is the one to do it; returns how many chats changed.
    fn see_off(&mut self, service: &mut Service) -> usize {
        let due = self.device.departures().expect("departures");
        for chat in &due {
            self.device.remove_departed(chat).expect("remove departed");
        }
        self.flush(service);
        due.len()
    }
}

fn names(received: &[Received]) -> Vec<String> {
    received
        .iter()
        .filter_map(|r| match r {
            Received::Message {
                message:
                    AppMessage {
                        body: Body::Name { name },
                        ..
                    },
                ..
            } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Alice starts "Pier crew" with Bob and Carol; Bob accepts and writes.
fn pier_crew(service: &mut Service) -> ([Person; 3], ChatId) {
    let mut alice = Person::join(service, "alice");
    let mut bob = Person::join(service, "bob");
    let mut carol = Person::join(service, "carol");
    let members = [
        Person::entry(service, "bob"),
        Person::entry(service, "carol"),
    ];
    let (chat, _) = alice
        .device
        .create_group("Pier crew", &members, &[])
        .expect("group");
    alice.flush(service);
    assert_eq!(alice.device.chat_name(&chat), Some("Pier crew"));

    for person in [&mut bob, &mut carol] {
        let got = person.read(service);
        assert!(matches!(&got[0], Received::Joined { chat: c, .. } if *c == chat));
        assert_eq!(names(&got), vec!["Pier crew".to_owned()]);
        let summary = &person.device.chat_list().expect("chats")[0];
        assert_eq!(summary.name.as_deref(), Some("Pier crew"));
        assert_eq!(summary.members.len(), 2, "the two others");
        assert!(summary.request, "nobody accepted yet");
    }

    bob.device.accept_chat(&chat).expect("accept");
    bob.device
        .send(&chat, &text(1, "Count me in"))
        .expect("send");
    bob.flush(service);
    for person in [&mut alice, &mut carol] {
        assert_eq!(
            texts(&person.read(service)),
            vec![(bob.author(), "Count me in".to_owned())]
        );
    }
    ([alice, bob, carol], chat)
}

#[test]
fn a_group_is_named_and_grows() {
    let mut service = Service::default();
    let ([mut alice, mut bob, mut carol], chat) = pier_crew(&mut service);
    let mut dave = Person::join(&mut service, "dave");

    // Alice renames it; the newest name wins everywhere.
    alice
        .device
        .rename_chat(&chat, "Pier crew 🌊")
        .expect("rename");
    alice.flush(&mut service);
    for person in [&mut bob, &mut carol] {
        assert_eq!(
            names(&person.read(&mut service)),
            vec!["Pier crew 🌊".to_owned()]
        );
        assert_eq!(person.device.chat_name(&chat), Some("Pier crew 🌊"));
    }

    // Bob adds Dave: Dave learns the name; the others see only the change.
    let invitation = bob
        .device
        .add_members(&chat, &[Person::entry(&mut service, "dave")])
        .expect("add");
    assert!(invitation.commit.commit.is_some(), "sequenced");
    assert_eq!(
        bob.flush(&mut service),
        vec![Settled::Applied, Settled::Delivered],
        "the change, then the name for Dave"
    );
    assert!(matches!(
        dave.read(&mut service)[0],
        Received::Joined { .. }
    ));
    assert_eq!(dave.device.chat_name(&chat), Some("Pier crew 🌊"));
    assert_eq!(dave.device.chat_list().expect("chats")[0].members.len(), 3);
    assert!(matches!(
        carol.read(&mut service)[0],
        Received::ChatUpdated { .. }
    ));
    let events = carol
        .device
        .history(&chat, None, 10)
        .expect("history")
        .iter()
        .filter(|entry| entry.message.content.is_event())
        .count();
    assert_eq!(events, 2, "two namings shown, not the repeat for Dave");
}

#[test]
fn people_leave_a_group_and_hear_nothing_more() {
    let mut service = Service::default();
    let ([mut alice, mut bob, mut carol], chat) = pier_crew(&mut service);

    // Carol leaves: she erases the chat; the member with the lowest leaf
    // takes her devices out, and only that one.
    carol.device.accept_chat(&chat).expect("accept");
    carol.device.leave_chat(&chat).expect("leave");
    carol.flush(&mut service);
    assert_eq!(carol.device.chats(), Vec::<ChatId>::new());
    for person in [&mut alice, &mut bob] {
        person.read(&mut service);
    }
    assert_eq!(bob.see_off(&mut service), 0);
    assert_eq!(
        alice.see_off(&mut service),
        1,
        "Alice holds the lowest leaf"
    );
    // Alice's pending receipts went out first; then her change.
    assert!(
        bob.read(&mut service)
            .iter()
            .any(|r| matches!(r, Received::ChatUpdated { .. }))
    );
    assert_eq!(bob.device.chat_list().expect("chats")[0].members.len(), 1);
    assert_eq!(alice.see_off(&mut service), 0, "nothing left to do");

    // Carol erased the chat: whatever still arrives for it is refused.
    let late = service.take(&carol.device);
    assert_ne!(late, Vec::<Vec<u8>>::new());
    for bytes in &late {
        let opened = carol.device.open_envelope(bytes).expect("envelope");
        assert!(matches!(opened.result, Err(CoreError::UnknownChat)));
    }

    // Bob writes; Carol is gone.
    bob.device.send(&chat, &text(2, "Just us")).expect("send");
    bob.flush(&mut service);
    assert_eq!(service.take(&carol.device), Vec::<Vec<u8>>::new());
    assert_eq!(
        texts(&alice.read(&mut service)),
        vec![(bob.author(), "Just us".to_owned())]
    );
}
