//! Drift D1 (ADR-0020): a message handed to a neighbour on the local
//! network is read like one from the service, and the copy the service
//! brings later is refused without a trace.

// An integration-test crate: helpers outside `#[test]` functions may fail
// loudly too, because a failed expectation is exactly what a test reports.
#![allow(clippy::expect_used, clippy::panic)]

use zelamar_core::content::{AppMessage, Body, MessageId};
use zelamar_core::device::{ChatId, Device, Received};
use zelamar_core::identity::AccountKey;
use zelamar_core::lan::{self, NONCE_LEN};
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
}

/// Alice starts a chat with Bob; the invitation stays in Alice's outbox,
/// as it would until the service orders it.
fn connect(alice: &mut Person, bob: &mut Person) -> ChatId {
    let chat = alice.device.create_chat().expect("chat");
    let key_package = bob.device.key_package().expect("key package");
    let invitation = alice.device.invite(&chat, &key_package).expect("invite");
    bob.device.join(&invitation.welcome.payload).expect("join");
    chat
}

#[test]
fn a_message_reaches_a_neighbour_and_its_second_copy_changes_nothing() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);
    assert_eq!(
        alice.device.parcels().expect("parcels"),
        Vec::<lan::Parcel>::new(),
        "a change to the chat's members, and its welcome, wait for the service"
    );

    let message = AppMessage::text(MessageId([1; 16]), 1_760_000_000_000, "No internet here");
    alice.device.send(&chat, &message).expect("send");
    let parcels = alice.device.parcels().expect("parcels");
    assert_eq!(parcels.len(), 1, "one envelope for Bob's one device");
    let parcel = &parcels[0];
    assert_eq!(parcel.mailbox, bob.device.mailbox());

    // Bob answers the nonce with his tag; Alice recognizes it.
    let nonce = [9; NONCE_LEN];
    assert_eq!(
        bob.device.own_tag(&nonce).expect("tag"),
        lan::tag(&parcel.mailbox, &nonce).expect("tag")
    );

    let opened = bob
        .device
        .open_envelope(&parcel.envelope)
        .expect("envelope");
    assert_eq!(opened.id, parcel.id);
    match opened.result.expect("read") {
        Received::Message { message, .. } => {
            assert_eq!(
                message.body,
                Body::Text {
                    text: "No internet here".into()
                }
            );
        }
        other => panic!("expected the message, got {other:?}"),
    }
    let before = bob.device.history(&chat, None, 10).expect("history");

    // The internet is back: the service brings the same envelope again.
    let again = bob
        .device
        .open_envelope(&parcel.envelope)
        .expect("envelope");
    assert!(again.result.is_err(), "already read: refused");
    assert_eq!(
        bob.device.history(&chat, None, 10).expect("history"),
        before,
        "nothing shows twice"
    );
    // Handed over nearby, it still waits for the service.
    assert!(
        alice
            .device
            .outbox()
            .expect("outbox")
            .iter()
            .any(|item| item.id == parcel.id)
    );
}
