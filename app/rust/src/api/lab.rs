//! Crypto Lab: two real Zelamar devices inside one app.
//!
//! Alice and Bob each get their own account, encrypted store and MLS state,
//! and talk through real envelopes. The lab exists so the CEO (and anyone
//! testing) can watch end-to-end encryption work, including across restarts,
//! long before the delivery service exists.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use zelamar_core::content::{AppMessage, Body, MessageId};
use zelamar_core::device::{ChatId, Device, Received};
use zelamar_core::envelope::{Envelope, EnvelopeId};
use zelamar_core::identity::{AccountId, AccountKey};
use zelamar_core::store::MasterKey;
use zelamar_core::CoreError;

/// Human name of the ciphersuite every Zelamar chat uses.
const CIPHERSUITE_NAME: &str = "MLS · ML-KEM-768 + X25519 · AES-256-GCM";

/// How many envelope bytes the lab shows as hex.
const PREVIEW_BYTES: usize = 48;

/// One of the two lab participants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabSide {
    Alice,
    Bob,
}

/// A participant as the UI shows it.
#[derive(Debug, Clone)]
pub struct LabParticipant {
    pub side: LabSide,
    pub name: String,
    /// Short, human-comparable form of the account key.
    pub fingerprint: String,
}

/// Everything that happened to one message on its way from one side to the
/// other.
#[derive(Debug, Clone)]
pub struct LabExchange {
    pub from: LabSide,
    /// What the sender typed.
    pub text: String,
    /// Size of the envelope on the wire.
    pub envelope_bytes: u32,
    /// The first bytes of the envelope, hex, grouped for reading.
    pub envelope_preview: String,
    pub ciphersuite: String,
    /// What the receiver decrypted.
    pub delivered_text: String,
    /// Whether the receiver authenticated the sender's account.
    pub author_verified: bool,
    /// Encrypt, seal, transmit, open, decrypt: wall time in microseconds.
    pub micros: u64,
}

/// A lab failure, shown to the user as-is.
#[derive(Debug)]
pub struct LabError {
    pub message: String,
}

impl From<CoreError> for LabError {
    fn from(error: CoreError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

struct Member {
    name: &'static str,
    path: PathBuf,
    master: MasterKey,
    account: AccountId,
    device: Device,
}

impl Member {
    fn create(name: &'static str, directory: &Path) -> Result<Self, LabError> {
        let path = directory.join(format!("{}.db", name.to_lowercase()));
        let account = AccountKey::generate()?;
        let master = MasterKey::generate()?;
        let device = Device::create(&path, &master, &account)?;
        Ok(Self {
            name,
            path,
            master,
            account: account.id(),
            device,
        })
    }

    fn participant(&self, side: LabSide) -> LabParticipant {
        LabParticipant {
            side,
            name: self.name.to_owned(),
            fingerprint: fingerprint(&self.account),
        }
    }
}

/// Two devices connected by one chat.
pub struct CryptoLab {
    alice: Member,
    bob: Member,
    chat: ChatId,
}

impl CryptoLab {
    /// Starts a fresh lab in `directory`, discarding any previous lab there.
    pub fn create(directory: String) -> Result<Self, LabError> {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).map_err(io_error)?;
        for name in ["alice", "bob"] {
            for suffix in ["db", "db-wal", "db-shm"] {
                let file = directory.join(format!("{name}.{suffix}"));
                if file.exists() {
                    std::fs::remove_file(&file).map_err(io_error)?;
                }
            }
        }

        let mut alice = Member::create("Alice", &directory)?;
        let mut bob = Member::create("Bob", &directory)?;
        let chat = alice.device.create_chat()?;
        let key_package = bob.device.key_package()?;
        let invitation = alice.device.invite(&chat, &key_package)?;
        bob.device.join(&invitation.welcome.payload)?;
        alice.device.confirm_sent(invitation.welcome.id)?;
        Ok(Self { alice, bob, chat })
    }

    /// Both participants, Alice first.
    #[flutter_rust_bridge::frb(sync)]
    #[must_use]
    pub fn participants(&self) -> Vec<LabParticipant> {
        vec![
            self.alice.participant(LabSide::Alice),
            self.bob.participant(LabSide::Bob),
        ]
    }

    /// Sends `text` from one side to the other through a real envelope. The
    /// lab plays the transport: it delivers to the address the sender's
    /// device chose, then confirms delivery so the outbox empties.
    pub fn send(&mut self, from: LabSide, text: String) -> Result<LabExchange, LabError> {
        let started = Instant::now();
        let message = AppMessage::text(MessageId(random_bytes()?), now_ms(), text.clone());
        let (sender, receiver) = match from {
            LabSide::Alice => (&mut self.alice, &mut self.bob),
            LabSide::Bob => (&mut self.bob, &mut self.alice),
        };

        let outgoing = sender.device.send(&self.chat, &message)?;
        let to = receiver.device.mailbox();
        if outgoing.recipients != [to] {
            return Err(LabError {
                message: "the message was not addressed to the receiver".to_owned(),
            });
        }
        let wire = Envelope {
            id: outgoing.id,
            to,
            payload: outgoing.payload,
        }
        .encode()?;
        let received = receiver.device.receive(&Envelope::decode(&wire)?.payload)?;
        sender.device.confirm_sent(outgoing.id)?;

        let (delivered_text, author_verified) = match received {
            Received::Message {
                author,
                message:
                    AppMessage {
                        body: Body::Text { text },
                        ..
                    },
                ..
            } => (text, author.account == sender.account),
            _ => {
                return Err(LabError {
                    message: "the receiver did not get a text message".to_owned(),
                })
            }
        };

        Ok(LabExchange {
            from,
            text,
            envelope_bytes: u32::try_from(wire.len()).unwrap_or(u32::MAX),
            envelope_preview: hex_preview(&wire),
            ciphersuite: CIPHERSUITE_NAME.to_owned(),
            delivered_text,
            author_verified,
            micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        })
    }

    /// Closes both devices and reopens them from their encrypted stores, as
    /// if the app had been restarted.
    pub fn restart(&mut self) -> Result<(), LabError> {
        for member in [&mut self.alice, &mut self.bob] {
            member.device = Device::open(&member.path, &member.master)?;
        }
        Ok(())
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], LabError> {
    // Envelope ids are 16 random bytes from the core's CSPRNG; longer
    // values are stitched from several of them.
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(16) {
        let id = EnvelopeId::random()?;
        chunk.copy_from_slice(&id.0[..chunk.len()]);
    }
    Ok(out)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `a1b2 c3d4 e5f6 0718`: the first 8 bytes of the account key.
fn fingerprint(account: &AccountId) -> String {
    group_hex(&account.as_bytes()[..8], 2)
}

fn hex_preview(bytes: &[u8]) -> String {
    let shown = &bytes[..bytes.len().min(PREVIEW_BYTES)];
    let mut out = group_hex(shown, 4);
    if bytes.len() > PREVIEW_BYTES {
        out.push_str(" …");
    }
    out
}

/// Hex in space-separated groups of `group` bytes.
fn group_hex(bytes: &[u8], group: usize) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && i % group == 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn io_error(error: std::io::Error) -> LabError {
    LabError {
        message: format!("lab storage: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab() -> (tempfile::TempDir, CryptoLab) {
        let dir = tempfile::tempdir().expect("temp dir");
        let lab = CryptoLab::create(dir.path().to_string_lossy().into_owned()).expect("lab");
        (dir, lab)
    }

    #[test]
    fn messages_flow_both_ways_with_verified_authors() {
        let (_dir, mut lab) = lab();
        let there = lab.send(LabSide::Alice, "Hello, Bob".into()).expect("send");
        assert_eq!(there.delivered_text, "Hello, Bob");
        assert!(there.author_verified);
        // "ZE" magic and version 0 open every envelope.
        assert!(there.envelope_preview.starts_with("5a4500"));
        let back = lab.send(LabSide::Bob, "Hi, Alice".into()).expect("send");
        assert_eq!(back.delivered_text, "Hi, Alice");
        assert!(back.author_verified);
    }

    #[test]
    fn the_conversation_survives_a_restart() {
        let (_dir, mut lab) = lab();
        lab.send(LabSide::Alice, "before".into()).expect("send");
        lab.restart().expect("restart");
        let after = lab.send(LabSide::Bob, "after".into()).expect("send");
        assert_eq!(after.delivered_text, "after");
    }

    #[test]
    fn everything_sent_is_delivered_confirmed_and_kept() {
        let (_dir, mut lab) = lab();
        lab.send(LabSide::Alice, "one".into()).expect("send");
        lab.send(LabSide::Bob, "two".into()).expect("send");
        lab.restart().expect("restart");
        for member in [&lab.alice, &lab.bob] {
            assert_eq!(member.device.outbox().expect("outbox").len(), 0);
            assert_eq!(
                member
                    .device
                    .history(&lab.chat, None, 10)
                    .expect("history")
                    .len(),
                2
            );
        }
    }

    #[test]
    fn participants_have_distinct_fingerprints() {
        let (_dir, lab) = lab();
        let people = lab.participants();
        assert_eq!(people.len(), 2);
        assert_ne!(people[0].fingerprint, people[1].fingerprint);
        assert_eq!(people[0].fingerprint.len(), "a1b2 c3d4 e5f6 0718".len());
    }

    #[test]
    fn hex_is_grouped_and_truncated() {
        assert_eq!(group_hex(&[0xab, 0xcd, 0xef], 2), "abcd ef");
        assert!(hex_preview(&[0; 100]).ends_with(" …"));
    }
}
