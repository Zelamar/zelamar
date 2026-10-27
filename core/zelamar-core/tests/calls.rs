//! Calls ride the chat (ADR-0019): the offer, the answer and the end are
//! MLS messages between the two people, and history keeps how the call
//! went, never the session descriptions.

// An integration-test crate: helpers outside `#[test]` functions may fail
// loudly too, because a failed expectation is exactly what a test reports.
#![allow(clippy::expect_used, clippy::panic)]

use zelamar_core::content::{AppMessage, Body, CallEndReason, MessageId};
use zelamar_core::device::{ChatId, Device, Received};
use zelamar_core::history::{CallRecord, StoredContent};
use zelamar_core::identity::AccountKey;
use zelamar_core::store::MasterKey;

struct Person {
    device: Device,
    _dir: tempfile::TempDir,
}

impl Person {
    fn new() -> Self {
        let account = AccountKey::generate().expect("account key");
        let master = MasterKey::generate().expect("master key");
        let dir = tempfile::tempdir().expect("temp dir");
        let device =
            Device::create(&dir.path().join("device.db"), &master, &account).expect("create");
        Self { device, _dir: dir }
    }

    /// Sends `body` as message `n` into `chat`; returns the ciphertext.
    fn say(&mut self, chat: &ChatId, n: u8, body: Body) -> Vec<u8> {
        let message = AppMessage::new(MessageId([n; 16]), 1_760_000_000_000 + u64::from(n), body);
        self.device.send(chat, &message).expect("send").payload
    }

    fn hear(&mut self, wire: &[u8]) -> Body {
        match self.device.receive(wire).expect("receive") {
            Received::Message { message, .. } => message.body,
            other => panic!("expected a message, got {other:?}"),
        }
    }

    fn call(&self, chat: &ChatId, n: u8) -> CallRecord {
        let record = self
            .device
            .message(chat, MessageId([n; 16]))
            .expect("history")
            .expect("the call is in history");
        match record.content {
            StoredContent::Call(call) => call,
            other => panic!("not a call: {other:?}"),
        }
    }

    fn unread(&self, chat: &ChatId) -> u32 {
        self.device
            .chat_list()
            .expect("chat list")
            .into_iter()
            .find(|summary| &summary.chat == chat)
            .expect("the chat is listed")
            .unread
    }
}

fn connect(alice: &mut Person, bob: &mut Person) -> ChatId {
    let chat = alice.device.create_chat().expect("chat");
    let key_package = bob.device.key_package().expect("key package");
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    bob.device.join(&invitation.welcome.payload).expect("join");
    chat
}

const OFFER: &str = "v=0\r\no=- 4611731400430051336 2 IN IP4 127.0.0.1\r\na=offer-marker\r\n";
const ANSWER: &str = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\na=answer-marker\r\n";

#[test]
fn a_call_is_offered_answered_and_ended_inside_the_chat() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);
    let call = MessageId([1; 16]);

    let wire = alice.say(
        &chat,
        1,
        Body::Call {
            video: true,
            sdp: OFFER.into(),
        },
    );
    assert!(
        !wire.windows(14).any(|w| w == b"a=offer-marker"),
        "the offer never travels in the clear"
    );
    assert_eq!(
        bob.hear(&wire),
        Body::Call {
            video: true,
            sdp: OFFER.into()
        }
    );
    assert_eq!(bob.unread(&chat), 1, "ringing: not seen yet");

    let wire = bob.say(
        &chat,
        2,
        Body::CallAnswer {
            call,
            sdp: ANSWER.into(),
        },
    );
    assert_eq!(bob.unread(&chat), 0, "picked up: nothing missed");
    assert_eq!(
        alice.hear(&wire),
        Body::CallAnswer {
            call,
            sdp: ANSWER.into()
        }
    );
    let answered = alice.call(&chat, 1);
    assert!(answered.video);
    assert!(answered.answered_by.is_some());
    assert_eq!(answered.end, None);

    let wire = alice.say(
        &chat,
        3,
        Body::CallEnd {
            call,
            reason: CallEndReason::Hangup,
            duration_ms: Some(204_000),
        },
    );
    bob.hear(&wire);
    for person in [&alice, &bob] {
        let ended = person.call(&chat, 1);
        assert_eq!(ended.end, Some(CallEndReason::Hangup));
        assert_eq!(ended.duration_ms, Some(204_000));
    }
    assert_eq!(bob.unread(&chat), 0);
}

#[test]
fn a_call_nobody_picks_up_is_missed() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);

    let wire = alice.say(
        &chat,
        1,
        Body::Call {
            video: false,
            sdp: OFFER.into(),
        },
    );
    bob.hear(&wire);
    let wire = alice.say(
        &chat,
        2,
        Body::CallEnd {
            call: MessageId([1; 16]),
            reason: CallEndReason::Missed,
            duration_ms: None,
        },
    );
    bob.hear(&wire);

    let missed = bob.call(&chat, 1);
    assert_eq!(missed.answered_by, None);
    assert_eq!(missed.end, Some(CallEndReason::Missed));
    assert_eq!(bob.unread(&chat), 1, "a missed call waits to be seen");
    assert_eq!(alice.unread(&chat), 0, "one's own call is never unread");

    bob.device.mark_read(&chat).expect("read");
    assert_eq!(bob.unread(&chat), 0);
}

#[test]
fn only_the_offer_wakes_a_phone_that_is_away() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);
    let offer = AppMessage::new(
        MessageId([1; 16]),
        1,
        Body::Call {
            video: false,
            sdp: OFFER.into(),
        },
    );
    assert!(alice.device.send(&chat, &offer).expect("send").urgent);
    let end = AppMessage::new(
        MessageId([2; 16]),
        2,
        Body::CallEnd {
            call: MessageId([1; 16]),
            reason: CallEndReason::Hangup,
            duration_ms: None,
        },
    );
    assert!(!alice.device.send(&chat, &end).expect("send").urgent);
    let text = AppMessage::text(MessageId([3; 16]), 3, "hi");
    assert!(!alice.device.send(&chat, &text).expect("send").urgent);
}
