//! Who is who: the other accounts this device talks to, and the usernames
//! they go by.
//!
//! A username reaches a device two ways. Looking someone up in the
//! directory confirms it. A profile message from the account itself only
//! claims it, until the directory confirms it too. A claim the directory
//! refutes (the name is someone else's, or no one's) is dropped for good:
//! that account is never shown by that name.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::identity::AccountId;

/// Another account, as this device knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// The account.
    pub account: AccountId,
    /// The username it goes by, if known.
    pub username: Option<String>,
    /// Whether the directory confirmed that `username` belongs to `account`.
    pub verified: bool,
    /// A name this account claimed and the directory refuted; its claims of
    /// that name are ignored from then on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refuted: Option<String>,
}

impl Contact {
    /// Someone known only by their account key.
    #[must_use]
    pub const fn unknown(account: AccountId) -> Self {
        Self {
            account,
            username: None,
            verified: false,
            refuted: None,
        }
    }
}

/// Every contact, by account.
pub(crate) type Contacts = BTreeMap<AccountId, Contact>;

/// `account` says it goes by `username`: remembered, unconfirmed, unless
/// it is the name already confirmed for that account.
pub(crate) fn claimed(contacts: &mut Contacts, account: AccountId, username: &str) {
    let contact = contacts
        .entry(account)
        .or_insert_with(|| Contact::unknown(account));
    if contact.refuted.as_deref() == Some(username) {
        return;
    }
    if contact.username.as_deref() != Some(username) {
        contact.username = Some(username.to_owned());
        contact.verified = false;
    }
}

/// The directory says `username` belongs to `account`.
pub(crate) fn confirmed(contacts: &mut Contacts, account: AccountId, username: &str) {
    contacts.insert(
        account,
        Contact {
            account,
            username: Some(username.to_owned()),
            verified: true,
            refuted: None,
        },
    );
}

/// The directory says `username` is not `account`'s: the claim is dropped
/// and ignored from now on.
pub(crate) fn refuted(contacts: &mut Contacts, account: AccountId, username: &str) {
    if let Some(contact) = contacts.get_mut(&account)
        && !contact.verified
        && contact.username.as_deref() == Some(username)
    {
        contact.username = None;
        contact.refuted = Some(username.to_owned());
    }
}

pub(crate) fn encode(contacts: &Contacts) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::into_writer(&contacts.values().collect::<Vec<_>>(), &mut out).map_err(|e| {
        CoreError::Encoding {
            what: "contacts",
            source: e.into(),
        }
    })?;
    Ok(out)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Contacts> {
    let list: Vec<Contact> = ciborium::from_reader(bytes)
        .map_err(|_| CoreError::malformed("contacts", "invalid encoding"))?;
    Ok(list
        .into_iter()
        .map(|contact| (contact.account, contact))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::AccountKey;

    #[test]
    fn a_claim_never_overrides_a_confirmation_of_the_same_name() {
        let alice = AccountKey::generate().expect("account").id();
        let mut contacts = Contacts::new();
        confirmed(&mut contacts, alice, "alice");
        claimed(&mut contacts, alice, "alice");
        assert!(contacts[&alice].verified);

        claimed(&mut contacts, alice, "alice_new");
        assert_eq!(contacts[&alice].username.as_deref(), Some("alice_new"));
        assert!(
            !contacts[&alice].verified,
            "a new name needs the directory again"
        );
    }

    #[test]
    fn contacts_round_trip() {
        let mut contacts = Contacts::new();
        for name in ["alice", "bob"] {
            let account = AccountKey::generate().expect("account").id();
            claimed(&mut contacts, account, name);
        }
        assert_eq!(
            decode(&encode(&contacts).expect("encode")).expect("decode"),
            contacts
        );
    }
}
