//! The local, encrypted history of each chat (ADR-0011).
//!
//! History is derived state: it is what the app shows. Every message, sent
//! or received, is folded into it in the same transaction as the MLS state
//! change that carried it.

use serde::{Deserialize, Serialize};

use crate::content::{AppMessage, Body, CallEndReason, FileRef, MessageId, ReceiptStatus};
use crate::device::Author;
use crate::error::{CoreError, Result};
use crate::identity::AccountId;

/// Storage format version of [`StoredMessage`].
const RECORD_VERSION: u16 = 0;

/// What a stored message says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredContent {
    /// Plain text.
    Text {
        /// The current text (after edits); empty once deleted.
        text: String,
    },
    /// Text answering an earlier message.
    Reply {
        /// The message answered.
        to: MessageId,
        /// The current text (after edits); empty once deleted.
        text: String,
    },
    /// A kind from a newer Zelamar; the app asks the user to update.
    Unknown {
        /// The kind's name.
        kind: String,
    },
    /// The author named the chat.
    Named {
        /// The new name.
        name: String,
    },
    /// The author left the chat.
    Left,
    /// A file.
    File(Box<FileRef>),
    /// A call (ADR-0019).
    Call(CallRecord),
}

impl StoredContent {
    /// Whether this records something that happened to the chat (a name,
    /// someone leaving) rather than a message.
    #[must_use]
    pub const fn is_event(&self) -> bool {
        matches!(self, Self::Named { .. } | Self::Left)
    }

    /// Whether this is a call someone picked up: nothing was missed.
    #[must_use]
    pub const fn is_answered_call(&self) -> bool {
        matches!(self, Self::Call(call) if call.answered_by.is_some())
    }
}

/// A call as it went (ADR-0019). The offer and answer themselves are never
/// kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallRecord {
    /// Whether it started with video.
    pub video: bool,
    /// The device that picked up, if one did.
    #[serde(default)]
    pub answered_by: Option<Author>,
    /// Why it ended, once it has.
    #[serde(default)]
    pub end: Option<CallEndReason>,
    /// How long the people talked, as whoever ended it measured.
    #[serde(default)]
    pub duration_ms: Option<u32>,
}

/// Where an outgoing message is on its way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendStatus {
    /// Encrypted and waiting in the outbox (for example, while offline).
    Queued,
    /// Accepted by the delivery service.
    Sent,
}

/// One message as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMessage {
    /// Record format version.
    pub version: u16,
    /// Author-chosen id.
    pub id: MessageId,
    /// Authenticated author.
    pub author: Author,
    /// Whether this device's account wrote it.
    pub outgoing: bool,
    /// Author's timestamp (UTC ms): what the app displays.
    pub sent_at_ms: u64,
    /// When this device stored it (UTC ms, local clock).
    pub received_at_ms: u64,
    /// The content.
    pub content: StoredContent,
    /// When the author last edited it, if ever.
    pub edited_at_ms: Option<u64>,
    /// Deleted for everyone by its author.
    pub deleted: bool,
    /// One reaction per account.
    pub reactions: Vec<(AccountId, String)>,
    /// Accounts that confirmed delivery (outgoing messages only).
    pub delivered_to: Vec<AccountId>,
    /// Accounts that read it (outgoing messages only).
    pub read_by: Vec<AccountId>,
    /// Outbox progress (outgoing messages only).
    pub status: SendStatus,
}

impl StoredMessage {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).map_err(|e| CoreError::Encoding {
            what: "stored message",
            source: e.into(),
        })?;
        Ok(out)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        let record: Self = ciborium::from_reader(bytes)
            .map_err(|_| CoreError::malformed("stored message", "invalid encoding"))?;
        if record.version > RECORD_VERSION {
            return Err(CoreError::UnsupportedVersion {
                what: "stored message",
                version: record.version,
            });
        }
        Ok(record)
    }
}

/// One message of a chat's history, with its place in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Position in this device's history of the chat (arrival order). Pass
    /// the smallest one seen as `before` to load the previous page.
    pub seq: i64,
    /// The message.
    pub message: StoredMessage,
}

/// Whether `body` adds a message to history (rather than changing one).
pub(crate) const fn inserts(body: &Body) -> bool {
    matches!(
        body,
        Body::Text { .. }
            | Body::Reply { .. }
            | Body::File(_)
            | Body::Call { .. }
            | Body::Unknown { .. }
    )
}

/// Read access to history during an operation (sees the operation's own,
/// not yet persisted writes first).
pub(crate) trait HistoryReader {
    fn load(&self, id: MessageId) -> Result<Option<StoredMessage>>;
}

/// Folds one message into history: returns the records to write.
///
/// `outgoing` is true when this device's account wrote `message`; a new
/// record starts with `status`.
pub(crate) fn apply(
    history: &impl HistoryReader,
    author: Author,
    outgoing: bool,
    message: &AppMessage,
    now_ms: u64,
    status: SendStatus,
) -> Result<Vec<StoredMessage>> {
    let new_record = |content| StoredMessage {
        version: RECORD_VERSION,
        id: message.id,
        author,
        outgoing,
        sent_at_ms: message.sent_at_ms,
        received_at_ms: now_ms,
        content,
        edited_at_ms: None,
        deleted: false,
        reactions: Vec::new(),
        delivered_to: Vec::new(),
        read_by: Vec::new(),
        status,
    };

    match &message.body {
        Body::Text { text } => insert(
            history,
            new_record(StoredContent::Text { text: text.clone() }),
        ),
        Body::Reply { to, text } => insert(
            history,
            new_record(StoredContent::Reply {
                to: *to,
                text: text.clone(),
            }),
        ),
        Body::Unknown { kind } => insert(
            history,
            new_record(StoredContent::Unknown { kind: kind.clone() }),
        ),
        Body::Name { name } => insert(
            history,
            new_record(StoredContent::Named { name: name.clone() }),
        ),
        Body::Leave => insert(history, new_record(StoredContent::Left)),
        Body::File(file) => insert(history, new_record(StoredContent::File(file.clone()))),
        Body::Call { video, .. } => insert(
            history,
            new_record(StoredContent::Call(CallRecord {
                video: *video,
                answered_by: None,
                end: None,
                duration_ms: None,
            })),
        ),
        Body::CallAnswer { call, .. } => answer_call(history, author, *call),
        Body::CallEnd {
            call,
            reason,
            duration_ms,
        } => end_call(history, author, *call, *reason, *duration_ms),
        // Profiles update contacts, not history.
        Body::Profile { .. } => Ok(Vec::new()),
        Body::Edit { target, text } => edit(history, author, *target, text, message.sent_at_ms),
        Body::Delete { target } => delete(history, author, *target),
        Body::React { target, reaction } => {
            let Some(mut record) = history.load(*target)? else {
                return Ok(Vec::new());
            };
            if record.deleted
                || record.content.is_event()
                || matches!(record.content, StoredContent::Call(_))
            {
                return Ok(Vec::new());
            }
            record
                .reactions
                .retain(|(account, _)| *account != author.account);
            if let Some(reaction) = reaction {
                record.reactions.push((author.account, reaction.clone()));
            }
            Ok(vec![record])
        }
        // Our own receipts change nothing in our own history.
        Body::Receipt { .. } if outgoing => Ok(Vec::new()),
        Body::Receipt { status, messages } => receipt(history, author, *status, messages),
    }
}

/// `author` replaces the text of `target`, one of their own messages, at
/// `at`.
fn edit(
    history: &impl HistoryReader,
    author: Author,
    target: MessageId,
    text: &str,
    at: u64,
) -> Result<Vec<StoredMessage>> {
    let Some(mut record) = own_target(history, target, author)? else {
        return Ok(Vec::new());
    };
    if record.deleted {
        return Ok(Vec::new());
    }
    match &mut record.content {
        StoredContent::Text { text: current } | StoredContent::Reply { text: current, .. } => {
            text.clone_into(current);
        }
        StoredContent::Unknown { .. }
        | StoredContent::Named { .. }
        | StoredContent::Left
        | StoredContent::File(_)
        | StoredContent::Call(_) => return Ok(Vec::new()),
    }
    record.edited_at_ms = Some(at);
    Ok(vec![record])
}

/// `author` deletes `target`, one of their own messages, for everyone.
fn delete(
    history: &impl HistoryReader,
    author: Author,
    target: MessageId,
) -> Result<Vec<StoredMessage>> {
    let Some(mut record) = own_target(history, target, author)? else {
        return Ok(Vec::new());
    };
    match &mut record.content {
        StoredContent::Text { text } | StoredContent::Reply { text, .. } => text.clear(),
        StoredContent::Unknown { .. } => {}
        // A deleted file keeps nothing that could fetch or open it.
        StoredContent::File(_) => {
            record.content = StoredContent::Text {
                text: String::new(),
            };
        }
        // What happened to the chat, and calls, stay in its history.
        StoredContent::Named { .. } | StoredContent::Left | StoredContent::Call(_) => {
            return Ok(Vec::new());
        }
    }
    record.deleted = true;
    record.reactions.clear();
    Ok(vec![record])
}

/// `author` picks up `call`: only someone called does, once, while the
/// call is on.
fn answer_call(
    history: &impl HistoryReader,
    author: Author,
    call: MessageId,
) -> Result<Vec<StoredMessage>> {
    let Some(mut record) = history.load(call)? else {
        return Ok(Vec::new());
    };
    let caller = record.author.account;
    let StoredContent::Call(details) = &mut record.content else {
        return Ok(Vec::new());
    };
    if author.account == caller || details.answered_by.is_some() || details.end.is_some() {
        return Ok(Vec::new());
    }
    details.answered_by = Some(author);
    Ok(vec![record])
}

/// `author` ends `call`, which then stays ended. Once someone picked up,
/// only the two people talking can end it.
fn end_call(
    history: &impl HistoryReader,
    author: Author,
    call: MessageId,
    reason: CallEndReason,
    duration_ms: Option<u32>,
) -> Result<Vec<StoredMessage>> {
    let Some(mut record) = history.load(call)? else {
        return Ok(Vec::new());
    };
    let caller = record.author.account;
    let StoredContent::Call(details) = &mut record.content else {
        return Ok(Vec::new());
    };
    if details.end.is_some() {
        return Ok(Vec::new());
    }
    if let Some(answerer) = details.answered_by
        && author.account != caller
        && author.account != answerer.account
    {
        return Ok(Vec::new());
    }
    details.end = Some(reason);
    // A call nobody picked up lasted nothing.
    details.duration_ms = details.answered_by.and(duration_ms);
    Ok(vec![record])
}

/// `author` got as far as `status` with `messages`: the outgoing ones
/// among them record it.
fn receipt(
    history: &impl HistoryReader,
    author: Author,
    status: ReceiptStatus,
    messages: &[MessageId],
) -> Result<Vec<StoredMessage>> {
    let mut writes = Vec::new();
    for id in messages {
        let Some(mut record) = history.load(*id)? else {
            continue;
        };
        if !record.outgoing {
            continue;
        }
        add_once(&mut record.delivered_to, author.account);
        if status == ReceiptStatus::Read {
            add_once(&mut record.read_by, author.account);
        }
        writes.push(record);
    }
    Ok(writes)
}

/// Inserts a new message, refusing an id already used in this chat.
fn insert(history: &impl HistoryReader, record: StoredMessage) -> Result<Vec<StoredMessage>> {
    if history.load(record.id)?.is_some() {
        return Err(CoreError::malformed(
            "message",
            "id already used in this chat",
        ));
    }
    Ok(vec![record])
}

/// The target of an edit or delete, only if `author`'s account wrote it.
fn own_target(
    history: &impl HistoryReader,
    target: MessageId,
    author: Author,
) -> Result<Option<StoredMessage>> {
    Ok(history
        .load(target)?
        .filter(|record| record.author.account == author.account))
}

fn add_once(accounts: &mut Vec<AccountId>, account: AccountId) {
    if !accounts.contains(&account) {
        accounts.push(account);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use crate::identity::{AccountKey, DeviceId};

    #[derive(Default)]
    struct Memory(RefCell<HashMap<MessageId, StoredMessage>>);

    impl HistoryReader for Memory {
        fn load(&self, id: MessageId) -> Result<Option<StoredMessage>> {
            Ok(self.0.borrow().get(&id).cloned())
        }
    }

    impl Memory {
        fn fold(&self, author: Author, outgoing: bool, body: Body, n: u8) {
            let message = AppMessage::new(MessageId([n; 16]), 1_000 + u64::from(n), body);
            let records =
                apply(self, author, outgoing, &message, 9_999, SendStatus::Sent).expect("apply");
            for record in records {
                self.0.borrow_mut().insert(record.id, record);
            }
        }

        fn get(&self, n: u8) -> StoredMessage {
            self.0.borrow()[&MessageId([n; 16])].clone()
        }
    }

    fn person() -> Author {
        Author {
            account: AccountKey::generate().expect("account").id(),
            device: DeviceId::random().expect("device"),
        }
    }

    fn text(t: &str) -> Body {
        Body::Text { text: t.into() }
    }

    #[test]
    fn edits_and_deletes_apply_only_to_the_authors_own_messages() {
        let (alice, mallory) = (person(), person());
        let history = Memory::default();
        history.fold(alice, false, text("original"), 1);

        let target = MessageId([1; 16]);
        history.fold(
            mallory,
            false,
            Body::Edit {
                target,
                text: "forged".into(),
            },
            2,
        );
        history.fold(mallory, false, Body::Delete { target }, 3);
        assert_eq!(
            history.get(1).content,
            StoredContent::Text {
                text: "original".into()
            }
        );
        assert!(!history.get(1).deleted);

        history.fold(
            alice,
            false,
            Body::Edit {
                target,
                text: "edited".into(),
            },
            4,
        );
        let edited = history.get(1);
        assert_eq!(
            edited.content,
            StoredContent::Text {
                text: "edited".into()
            }
        );
        assert_eq!(edited.edited_at_ms, Some(1_004));

        history.fold(alice, false, Body::Delete { target }, 5);
        let deleted = history.get(1);
        assert!(deleted.deleted);
        assert_eq!(
            deleted.content,
            StoredContent::Text {
                text: String::new()
            }
        );
    }

    #[test]
    fn reactions_are_one_per_account_and_can_be_cleared() {
        let (alice, bob) = (person(), person());
        let history = Memory::default();
        history.fold(alice, false, text("hi"), 1);
        let target = MessageId([1; 16]);

        history.fold(
            bob,
            false,
            Body::React {
                target,
                reaction: Some("🙂".into()),
            },
            2,
        );
        history.fold(
            bob,
            false,
            Body::React {
                target,
                reaction: Some("❤️".into()),
            },
            3,
        );
        assert_eq!(history.get(1).reactions, vec![(bob.account, "❤️".into())]);

        history.fold(
            bob,
            false,
            Body::React {
                target,
                reaction: None,
            },
            4,
        );
        assert_eq!(history.get(1).reactions, Vec::<(AccountId, String)>::new());
    }

    #[test]
    fn receipts_update_only_outgoing_messages() {
        let (me, bob) = (person(), person());
        let history = Memory::default();
        history.fold(me, true, text("mine"), 1);
        history.fold(bob, false, text("theirs"), 2);

        let both = vec![MessageId([1; 16]), MessageId([2; 16])];
        history.fold(
            bob,
            false,
            Body::Receipt {
                status: ReceiptStatus::Read,
                messages: both,
            },
            3,
        );

        let mine = history.get(1);
        assert_eq!(mine.delivered_to, vec![bob.account]);
        assert_eq!(mine.read_by, vec![bob.account]);
        assert_eq!(history.get(2).read_by, Vec::<AccountId>::new());
    }

    fn call_of(history: &Memory, n: u8) -> CallRecord {
        match history.get(n).content {
            StoredContent::Call(call) => call,
            other => panic!("not a call: {other:?}"),
        }
    }

    fn call_end(reason: CallEndReason, duration_ms: Option<u32>) -> Body {
        Body::CallEnd {
            call: MessageId([1; 16]),
            reason,
            duration_ms,
        }
    }

    #[test]
    fn a_call_is_picked_up_once_by_someone_called_then_ended_by_a_speaker() {
        let (alice, bob, bobs_phone, carol) = (person(), person(), person(), person());
        let bobs_phone = Author {
            account: bob.account,
            ..bobs_phone
        };
        let history = Memory::default();
        history.fold(
            alice,
            false,
            Body::Call {
                video: false,
                sdp: "offer".into(),
            },
            1,
        );
        let answer = |sdp: &str| Body::CallAnswer {
            call: MessageId([1; 16]),
            sdp: sdp.into(),
        };

        // The caller cannot pick up their own call.
        history.fold(alice, false, answer("self"), 2);
        assert_eq!(call_of(&history, 1).answered_by, None);

        history.fold(bob, false, answer("bob"), 3);
        // Bob's other device is too late, and so is anyone else.
        history.fold(bobs_phone, false, answer("phone"), 4);
        history.fold(carol, false, answer("carol"), 5);
        assert_eq!(call_of(&history, 1).answered_by, Some(bob));
        assert!(
            history.get(1).content.is_answered_call(),
            "picked up: not missed"
        );

        // Someone else cannot end a call two people are having.
        history.fold(carol, false, call_end(CallEndReason::Hangup, Some(1)), 6);
        assert_eq!(call_of(&history, 1).end, None);

        history.fold(
            bob,
            false,
            call_end(CallEndReason::Hangup, Some(204_000)),
            7,
        );
        history.fold(alice, false, call_end(CallEndReason::Failed, Some(9)), 8);
        let ended = call_of(&history, 1);
        assert_eq!(ended.end, Some(CallEndReason::Hangup), "the first end wins");
        assert_eq!(ended.duration_ms, Some(204_000));

        // An ended call is not picked up later.
        history.fold(
            alice,
            false,
            Body::Call {
                video: true,
                sdp: "offer".into(),
            },
            9,
        );
        history.fold(
            bob,
            false,
            Body::CallEnd {
                call: MessageId([9; 16]),
                reason: CallEndReason::Declined,
                duration_ms: Some(5),
            },
            10,
        );
        history.fold(
            bob,
            false,
            Body::CallAnswer {
                call: MessageId([9; 16]),
                sdp: "late".into(),
            },
            11,
        );
        let declined = call_of(&history, 9);
        assert_eq!(declined.answered_by, None);
        assert_eq!(declined.end, Some(CallEndReason::Declined));
        assert_eq!(declined.duration_ms, None, "never picked up: no length");
        assert!(declined.video);
    }

    #[test]
    fn calls_are_not_edited_deleted_or_reacted_to() {
        let (alice, bob) = (person(), person());
        let history = Memory::default();
        history.fold(
            alice,
            false,
            Body::Call {
                video: false,
                sdp: "offer".into(),
            },
            1,
        );
        let target = MessageId([1; 16]);
        history.fold(
            alice,
            false,
            Body::Edit {
                target,
                text: "x".into(),
            },
            2,
        );
        history.fold(alice, false, Body::Delete { target }, 3);
        history.fold(
            bob,
            false,
            Body::React {
                target,
                reaction: Some("🙂".into()),
            },
            4,
        );
        let call = history.get(1);
        assert!(!call.deleted);
        assert_eq!(call.reactions, Vec::<(AccountId, String)>::new());
        assert_eq!(call.edited_at_ms, None);
        // An answer or end for something that is not a call does nothing.
        history.fold(alice, false, text("hi"), 5);
        history.fold(
            bob,
            false,
            Body::CallAnswer {
                call: MessageId([5; 16]),
                sdp: "x".into(),
            },
            6,
        );
        assert_eq!(
            history.get(5).content,
            StoredContent::Text { text: "hi".into() }
        );
    }

    #[test]
    fn a_reused_message_id_is_refused() {
        let (alice, mallory) = (person(), person());
        let history = Memory::default();
        history.fold(alice, false, text("genuine"), 1);
        let replay = AppMessage::new(MessageId([1; 16]), 5, text("impostor"));
        assert!(apply(&history, mallory, false, &replay, 0, SendStatus::Sent).is_err());
    }

    #[test]
    fn records_round_trip() {
        let history = Memory::default();
        history.fold(
            person(),
            true,
            Body::Reply {
                to: MessageId([7; 16]),
                text: "yes".into(),
            },
            1,
        );
        let record = history.get(1);
        assert_eq!(
            StoredMessage::decode(&record.encode().expect("encode")).expect("decode"),
            record
        );
    }
}
