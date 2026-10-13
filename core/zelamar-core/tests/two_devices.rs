//! End-to-end behaviour of the core through its public API only: two (and
//! three) devices chatting over MLS, wrapped in envelopes like a transport
//! would carry them.

// An integration-test crate: helpers outside `#[test]` functions may fail
// loudly too, because a failed expectation is exactly what a test reports.
#![allow(clippy::expect_used, clippy::panic)]

use zelamar_core::CoreError;
use zelamar_core::content::{AppMessage, Body, MessageId};
use zelamar_core::device::{Author, ChatId, Device, Received};
use zelamar_core::envelope::{Envelope, EnvelopeId, MailboxId};
use zelamar_core::identity::AccountKey;

struct Person {
    account: AccountKey,
    device: Device,
}

impl Person {
    fn new() -> Self {
        let account = AccountKey::generate().expect("account key");
        let device = Device::new(&account).expect("device");
        Self { account, device }
    }

    const fn author(&self) -> Author {
        Author {
            account: self.account.id(),
            device: self.device.id(),
        }
    }
}

/// Seals MLS bytes into an envelope and opens it again, like a transport.
fn over_the_wire(payload: Vec<u8>) -> Vec<u8> {
    let sealed = Envelope {
        id: EnvelopeId::random().expect("envelope id"),
        to: MailboxId([7; 32]),
        payload,
    }
    .encode()
    .expect("encode envelope");
    Envelope::decode(&sealed).expect("decode envelope").payload
}

fn text(n: u8, body: &str) -> AppMessage {
    AppMessage::text(MessageId([n; 16]), 1_760_000_000_000 + u64::from(n), body)
}

/// Alice creates a chat and invites Bob; returns the chat id.
fn alice_invites_bob(alice: &mut Person, bob: &mut Person) -> ChatId {
    let chat = alice.device.create_chat().expect("create chat");
    let key_package = over_the_wire(bob.device.key_package().expect("key package"));
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    let joined = bob
        .device
        .join(&over_the_wire(invitation.welcome))
        .expect("join");
    assert_eq!(joined, chat);
    chat
}

#[test]
fn two_devices_exchange_authenticated_messages() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);

    let hello = text(1, "Are we still on for tonight?");
    let wire = alice.device.send(&chat, &hello).expect("send");
    assert_eq!(
        bob.device.receive(&over_the_wire(wire)).expect("receive"),
        Received::Message {
            chat: chat.clone(),
            author: alice.author(),
            message: hello,
        }
    );

    let reply = text(2, "Yes! Where should we meet?");
    let wire = bob.device.send(&chat, &reply).expect("send");
    assert_eq!(
        alice.device.receive(&over_the_wire(wire)).expect("receive"),
        Received::Message {
            chat,
            author: bob.author(),
            message: reply,
        }
    );
}

#[test]
fn ciphertext_does_not_contain_the_plaintext() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);

    let secret = "the pier, near the old lighthouse";
    let wire = alice.device.send(&chat, &text(1, secret)).expect("send");
    assert!(
        !wire
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    );
}

#[test]
fn a_tampered_message_is_rejected_not_shown() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);

    let mut wire = alice.device.send(&chat, &text(1, "hi")).expect("send");
    let last = wire.len() - 1;
    wire[last] ^= 0x01;
    assert!(bob.device.receive(&wire).is_err());
}

#[test]
fn a_newcomer_cannot_read_messages_sent_before_joining() {
    let (mut alice, mut bob, mut carol) = (Person::new(), Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);

    let before = alice
        .device
        .send(&chat, &text(1, "before Carol"))
        .expect("send");

    let key_package = carol.device.key_package().expect("key package");
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    assert_eq!(
        bob.device.receive(&invitation.commit).expect("commit"),
        Received::ChatUpdated { chat: chat.clone() }
    );
    carol.device.join(&invitation.welcome).expect("join");

    // Forward secrecy for newcomers: the old epoch's keys are not Carol's.
    assert!(carol.device.receive(&before).is_err());

    // But everything sent after she joined reaches all three.
    let after = text(2, "welcome, Carol");
    let wire = alice.device.send(&chat, &after).expect("send");
    for reader in [&mut bob.device, &mut carol.device] {
        match reader.receive(&wire).expect("receive") {
            Received::Message { message, .. } => assert_eq!(message.body, after.body),
            other => panic!("expected a message, got {other:?}"),
        }
    }
}

#[test]
fn messages_for_unknown_chats_are_refused() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice.device.create_chat().expect("create chat");
    let wire = alice.device.send(&chat, &text(1, "hi")).expect("send");
    assert!(matches!(
        bob.device.receive(&wire),
        Err(CoreError::UnknownChat)
    ));
}

#[test]
fn garbage_is_never_mistaken_for_a_message() {
    let mut bob = Person::new();
    for junk in [&b""[..], b"ZE", &[0u8; 64], &[0xFFu8; 300]] {
        assert!(bob.device.receive(junk).is_err());
        assert!(bob.device.join(junk).is_err());
    }
    // Text bodies keep their kind through the whole pipeline.
    assert!(matches!(text(1, "x").body, Body::Text { .. }));
}
