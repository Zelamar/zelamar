//! Whatever arrives from the network, the core answers with a value or an
//! error: it never panics, and a refusal changes nothing. Every decoder that
//! reads bytes from outside the device is fed arbitrary and mangled input
//! here.

// An integration-test crate: helpers may fail loudly, as a test does.
#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use zelamar_core::attachment;
use zelamar_core::content::{AppMessage, MessageId};
use zelamar_core::device::{ChatId, Device};
use zelamar_core::envelope::{Envelope, EnvelopeId, MailboxId};
use zelamar_core::identity::{AccountKey, DeviceCertificate};
use zelamar_core::link::{Joining, Offer};
use zelamar_core::service;
use zelamar_core::store::MasterKey;

struct Pair {
    alice: Device,
    bob: Device,
    chat: ChatId,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// Two devices in one chat, with one message already exchanged.
fn pair() -> Pair {
    let make = || {
        let dir = tempfile::tempdir().expect("dir");
        let device = Device::create(
            &dir.path().join("device.db"),
            &MasterKey::generate().expect("key"),
            &AccountKey::generate().expect("account"),
        )
        .expect("device");
        (device, dir)
    };
    let (mut alice, alice_dir) = make();
    let (mut bob, bob_dir) = make();
    let chat = alice.create_chat().expect("chat");
    let invitation = alice
        .invite(&chat, &bob.key_package().expect("key package"))
        .expect("invite");
    bob.join(&invitation.welcome.payload).expect("join");
    let hello = alice
        .send(&chat, &AppMessage::text(MessageId([1; 16]), 1, "hello"))
        .expect("send");
    bob.receive(&hello.payload).expect("receive");
    Pair {
        alice,
        bob,
        chat,
        _dirs: (alice_dir, bob_dir),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn envelopes_certificates_and_answers_never_panic(
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let _ = Envelope::decode(&bytes);
        let _ = DeviceCertificate::decode(&bytes);
        let _ = service::directory_entry("alice", 200, &bytes);
        let _ = service::device_list("alice", 200, &bytes);
        let _ = service::commit_answer(409, &bytes);
        let _ = service::delivered(202, &bytes);
        let _ = attachment::downloaded(404, &bytes);
        let offer = Offer::new().expect("offer");
        let _ = offer.applicant(200, &bytes);
    }

    #[test]
    fn typed_link_codes_never_panic(text in "\\PC{0,40}") {
        let _ = Joining::new(&text, "Phone");
    }

    #[test]
    fn mangled_file_chunks_never_open(
        flip in 0usize..1040,
        size in 1usize..3000,
    ) {
        let plaintext = vec![0x5A; size];
        let sealed = attachment::seal_file(&plaintext).expect("seal");
        let mut chunks = sealed.chunks.clone();
        let at = flip % chunks[0].len();
        chunks[0][at] ^= 0x80;
        prop_assert!(attachment::open_file(&sealed.key, sealed.size, &chunks).is_err());
    }
}

/// A device fed garbage or mangled ciphertext refuses it and stays able to
/// read the next genuine message.
#[test]
fn a_device_survives_any_bytes_and_keeps_its_chat() {
    let mut p = pair();
    let genuine = p
        .alice
        .send(
            &p.chat,
            &AppMessage::text(MessageId([2; 16]), 2, "still here"),
        )
        .expect("send")
        .payload;

    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(128));
    let bob = std::cell::RefCell::new(&mut p.bob);
    runner
        .run(
            &(
                proptest::collection::vec(any::<u8>(), 0..600),
                any::<usize>(),
                any::<u8>(),
            ),
            |(noise, at, xor)| {
                let mut bob = bob.borrow_mut();
                prop_assert!(bob.receive(&noise).is_err());
                let mut mangled = genuine.clone();
                let index = at % mangled.len();
                mangled[index] ^= xor | 1;
                // A flipped bit anywhere either fails to decode or fails to
                // authenticate; it never turns into a different message.
                prop_assert!(bob.receive(&mangled).is_err());
                let envelope = Envelope {
                    id: EnvelopeId([9; 16]),
                    to: bob.mailbox(),
                    payload: noise,
                };
                let opened = bob.open_envelope(&envelope.encode().expect("envelope"));
                prop_assert!(opened.map_or(true, |o| o.result.is_err()));
                let elsewhere = Envelope {
                    id: EnvelopeId([9; 16]),
                    to: MailboxId([1; 32]),
                    payload: genuine.clone(),
                };
                let opened = bob
                    .open_envelope(&elsewhere.encode().expect("envelope"))
                    .expect("an envelope");
                prop_assert!(opened.result.is_err(), "not addressed to Bob");
                Ok(())
            },
        )
        .expect("garbage is refused");

    let received = p.bob.receive(&genuine).expect("the genuine message");
    assert!(matches!(
        received,
        zelamar_core::device::Received::Message { .. }
    ));
    assert_eq!(p.bob.history(&p.chat, None, 10).expect("history").len(), 2);
}
