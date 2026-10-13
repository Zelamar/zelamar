//! A device survives restarts, keeps nothing readable on disk, and is never
//! left half-way through a state change.

// An integration-test crate: helpers outside `#[test]` functions may fail
// loudly too, because a failed expectation is exactly what a test reports.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use zelamar_core::CoreError;
use zelamar_core::content::{AppMessage, MessageId};
use zelamar_core::device::{ChatId, Device, Received};
use zelamar_core::identity::AccountKey;
use zelamar_core::store::MasterKey;

/// A person whose device can be "restarted": dropped and reopened.
struct Person {
    account: AccountKey,
    master: MasterKey,
    path: PathBuf,
    device: Option<Device>,
    dir: tempfile::TempDir,
}

impl Person {
    fn new() -> Self {
        let account = AccountKey::generate().expect("account key");
        let master = MasterKey::generate().expect("master key");
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("device.db");
        let device = Device::create(&path, &master, &account).expect("create");
        Self {
            account,
            master,
            path,
            device: Some(device),
            dir,
        }
    }

    const fn device(&mut self) -> &mut Device {
        self.device.as_mut().expect("device is open")
    }

    fn restart(&mut self) {
        drop(self.device.take());
        self.device = Some(Device::open(&self.path, &self.master).expect("reopen"));
    }

    /// Every byte the device has written to disk (database, WAL, journal).
    fn bytes_on_disk(&self) -> Vec<u8> {
        files_in(self.dir.path())
            .into_iter()
            .flat_map(|file| std::fs::read(file).expect("read file"))
            .collect()
    }
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .expect("list dir")
        .map(|entry| entry.expect("entry").path())
        .collect()
}

fn text(n: u8, body: &str) -> AppMessage {
    AppMessage::text(MessageId([n; 16]), 1_760_000_000_000 + u64::from(n), body)
}

fn connect(alice: &mut Person, bob: &mut Person) -> ChatId {
    let chat = alice.device().create_chat().expect("chat");
    let key_package = bob.device().key_package().expect("key package");
    let invitation = alice.device().invite(&chat, &key_package).expect("invite");
    bob.device().join(&invitation.welcome).expect("join");
    chat
}

fn expect_text(received: Received) -> String {
    match received {
        Received::Message { message, .. } => match message.body {
            zelamar_core::content::Body::Text { text } => text,
            other => panic!("expected text, got {other:?}"),
        },
        other => panic!("expected a message, got {other:?}"),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn a_conversation_continues_across_restarts() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);

    let wire = alice
        .device()
        .send(&chat, &text(1, "before"))
        .expect("send");
    assert_eq!(
        expect_text(bob.device().receive(&wire).expect("receive")),
        "before"
    );

    alice.restart();
    bob.restart();

    let wire = bob
        .device()
        .send(&chat, &text(2, "after restart"))
        .expect("send");
    let received = alice.device().receive(&wire).expect("receive");
    match &received {
        Received::Message { author, .. } => assert_eq!(author.account, bob.account.id()),
        other => panic!("expected a message, got {other:?}"),
    }
    assert_eq!(expect_text(received), "after restart");

    let wire = alice
        .device()
        .send(&chat, &text(3, "and back"))
        .expect("send");
    assert_eq!(
        expect_text(bob.device().receive(&wire).expect("receive")),
        "and back"
    );
}

#[test]
fn a_key_package_survives_a_restart_of_its_owner() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = alice.device().create_chat().expect("chat");
    let key_package = bob.device().key_package().expect("key package");

    // Bob goes offline (restarts) before Alice invites him.
    bob.restart();

    let invitation = alice.device().invite(&chat, &key_package).expect("invite");
    assert_eq!(bob.device().join(&invitation.welcome).expect("join"), chat);
}

#[test]
fn nothing_readable_is_left_on_disk() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);
    let secret = "the pier, near the old lighthouse";
    let wire = alice.device().send(&chat, &text(1, secret)).expect("send");
    bob.device().receive(&wire).expect("receive");

    for person in [&alice, &bob] {
        let disk = person.bytes_on_disk();
        assert_ne!(disk.len(), 0, "the device wrote nothing to disk");
        assert!(!contains(&disk, secret.as_bytes()), "message text on disk");
        assert!(!contains(&disk, chat.as_bytes()), "chat id on disk");
        assert!(
            !contains(&disk, alice.account.id().as_bytes()),
            "account id on disk"
        );
        assert!(
            !contains(&disk, b"device.certificate"),
            "record name on disk"
        );
    }
}

#[test]
fn the_wrong_master_key_opens_nothing() {
    let mut alice = Person::new();
    drop(alice.device.take());
    let stranger = MasterKey::generate().expect("master key");
    assert!(matches!(
        Device::open(&alice.path, &stranger),
        Err(CoreError::WrongKey)
    ));
}

#[test]
fn a_rejected_message_changes_nothing_even_after_restart() {
    let (mut alice, mut bob) = (Person::new(), Person::new());
    let chat = connect(&mut alice, &mut bob);

    let mut forged = alice.device().send(&chat, &text(1, "real")).expect("send");
    let genuine = forged.clone();
    let last = forged.len() - 1;
    forged[last] ^= 1;
    assert!(bob.device().receive(&forged).is_err());

    bob.restart();
    assert_eq!(
        expect_text(bob.device().receive(&genuine).expect("receive")),
        "real"
    );
}

#[test]
fn opening_an_empty_store_reports_no_device() {
    let dir = tempfile::tempdir().expect("temp dir");
    let master = MasterKey::generate().expect("master key");
    assert!(matches!(
        Device::open(&dir.path().join("empty.db"), &master),
        Err(CoreError::NotInitialized)
    ));
}
