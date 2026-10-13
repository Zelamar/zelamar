//! End-to-end behaviour of the core through its public API only: two (and
//! three) devices chatting over MLS, wrapped in envelopes like a transport
//! would carry them.

// An integration-test crate: helpers outside `#[test]` functions may fail
// loudly too, because a failed expectation is exactly what a test reports.
#![allow(clippy::expect_used, clippy::panic)]

use zelamar_core::CoreError;
use zelamar_core::content::{AppMessage, Body, MessageId, ReceiptStatus};
use zelamar_core::device::{Author, ChatId, Device, Received};
use zelamar_core::envelope::{Envelope, EnvelopeId, MailboxId};
use zelamar_core::history::StoredContent;
use zelamar_core::identity::AccountKey;
use zelamar_core::store::MasterKey;

struct Person {
    account: AccountKey,
    device: Device,
    _dir: tempfile::TempDir,
}

impl Person {
    fn new() -> Self {
        let account = AccountKey::generate().expect("account key");
        let dir = tempfile::tempdir().expect("temp dir");
        let master = MasterKey::generate().expect("master key");
        let device =
            Device::create(&dir.path().join("device.db"), &master, &account).expect("device");
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

/// `from` sends message number `n` with `body`; `to` receives it.
fn deliver(from: &mut Device, to: &mut Device, chat: &ChatId, n: u8, body: Body) {
    let message = AppMessage::new(MessageId([n; 16]), 1_760_000_000_000 + u64::from(n), body);
    let outgoing = from.send(chat, &message).expect("send");
    to.receive(&over_the_wire(outgoing.payload))
        .expect("receive");
}

/// Alice creates a chat and invites Bob; returns the chat id.
fn alice_invites_bob(alice: &mut Person, bob: &mut Person) -> ChatId {
    let chat = alice.device.create_chat().expect("create chat");
    let key_package = over_the_wire(bob.device.key_package().expect("key package"));
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    let joined = bob
        .device
        .join(&over_the_wire(invitation.welcome.payload))
        .expect("join");
    assert_eq!(joined, chat);
    chat
}

#[test]
fn two_devices_exchange_authenticated_messages() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);

    let hello = text(1, "Are we still on for tonight?");
    let wire = alice.device.send(&chat, &hello).expect("send").payload;
    assert_eq!(
        bob.device.receive(&over_the_wire(wire)).expect("receive"),
        Received::Message {
            chat: chat.clone(),
            author: alice.author(),
            message: hello,
        }
    );

    let reply = text(2, "Yes! Where should we meet?");
    let wire = bob.device.send(&chat, &reply).expect("send").payload;
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
    let wire = alice
        .device
        .send(&chat, &text(1, secret))
        .expect("send")
        .payload;
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

    let mut wire = alice
        .device
        .send(&chat, &text(1, "hi"))
        .expect("send")
        .payload;
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
        .expect("send")
        .payload;

    let key_package = carol.device.key_package().expect("key package");
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    assert_eq!(
        bob.device
            .receive(&invitation.commit.payload)
            .expect("commit"),
        Received::ChatUpdated { chat: chat.clone() }
    );
    carol
        .device
        .join(&invitation.welcome.payload)
        .expect("join");

    // Forward secrecy for newcomers: the old epoch's keys are not Carol's.
    assert!(carol.device.receive(&before).is_err());

    // But everything sent after she joined reaches all three.
    let after = text(2, "welcome, Carol");
    let wire = alice.device.send(&chat, &after).expect("send").payload;
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
    let wire = alice
        .device
        .send(&chat, &text(1, "hi"))
        .expect("send")
        .payload;
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

#[test]
fn messages_are_addressed_to_every_other_members_mailbox() {
    let (mut alice, mut bob, mut carol) = (Person::new(), Person::new(), Person::new());
    let chat = alice.device.create_chat().expect("create chat");

    let key_package = bob.device.key_package().expect("key package");
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    assert_eq!(
        invitation.commit.recipients,
        Vec::<MailboxId>::new(),
        "no one else to tell"
    );
    assert_eq!(invitation.welcome.recipients, vec![bob.device.mailbox()]);
    bob.device.join(&invitation.welcome.payload).expect("join");

    let key_package = carol.device.key_package().expect("key package");
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    assert_eq!(invitation.commit.recipients, vec![bob.device.mailbox()]);
    assert_eq!(invitation.welcome.recipients, vec![carol.device.mailbox()]);
    bob.device
        .receive(&invitation.commit.payload)
        .expect("commit");
    carol
        .device
        .join(&invitation.welcome.payload)
        .expect("join");

    let mut others = vec![alice.device.mailbox(), carol.device.mailbox()];
    others.sort_unstable();
    let outgoing = bob.device.send(&chat, &text(1, "hi both")).expect("send");
    assert_eq!(outgoing.recipients, others);
}

#[test]
fn history_follows_the_conversation() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);
    let target = MessageId([1; 16]);
    let (a, b) = (&mut alice.device, &mut bob.device);

    deliver(
        a,
        b,
        &chat,
        1,
        Body::Text {
            text: "Dinner at eight?".into(),
        },
    );
    deliver(
        b,
        a,
        &chat,
        2,
        Body::React {
            target,
            reaction: Some("❤️".into()),
        },
    );
    deliver(
        b,
        a,
        &chat,
        3,
        Body::Receipt {
            status: ReceiptStatus::Read,
            messages: vec![target],
        },
    );
    deliver(
        a,
        b,
        &chat,
        4,
        Body::Edit {
            target,
            text: "Dinner at nine?".into(),
        },
    );
    // Bob cannot rewrite Alice's words: the edit arrives and changes nothing.
    deliver(
        b,
        a,
        &chat,
        5,
        Body::Edit {
            target,
            text: "Dinner is off".into(),
        },
    );

    let edited = StoredContent::Text {
        text: "Dinner at nine?".into(),
    };
    let alices = alice.device.history(&chat, None, 10).expect("history");
    assert_eq!(alices.len(), 1);
    let sent = &alices[0].message;
    assert!(sent.outgoing);
    assert_eq!(sent.author, alice.author());
    assert_eq!(sent.content, edited);
    assert_eq!(sent.edited_at_ms, Some(1_760_000_000_004));
    assert_eq!(sent.reactions, vec![(bob.account.id(), "❤️".to_owned())]);
    assert_eq!(sent.read_by, vec![bob.account.id()]);
    assert_eq!(sent.delivered_to, vec![bob.account.id()]);

    let bobs = bob.device.history(&chat, None, 10).expect("history");
    assert_eq!(bobs.len(), 1);
    let got = &bobs[0].message;
    assert!(!got.outgoing);
    assert_eq!(got.content, edited);
    assert_eq!(got.reactions, vec![(bob.account.id(), "❤️".to_owned())]);
}

#[test]
fn a_reused_message_id_is_dropped_without_a_trace() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice_invites_bob(&mut alice, &mut bob);

    alice.device.send(&chat, &text(1, "genuine")).expect("send");
    // Bob's (buggy or malicious) app reuses the id of Alice's message.
    let impostor = bob.device.send(&chat, &text(1, "impostor")).expect("send");
    assert!(alice.device.receive(&impostor.payload).is_err());

    let history = alice.device.history(&chat, None, 10).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].message.content,
        StoredContent::Text {
            text: "genuine".into()
        }
    );

    // The refusal rolled everything back; the chat goes on.
    let (a, b) = (&mut alice.device, &mut bob.device);
    deliver(
        b,
        a,
        &chat,
        2,
        Body::Text {
            text: "sorry".into(),
        },
    );
    assert_eq!(
        alice
            .device
            .history(&chat, None, 10)
            .expect("history")
            .len(),
        2
    );
}
