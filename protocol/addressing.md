# Addressing — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/device.rs`. Rationale: ADR-0011.

A sender finds every recipient device in the chat itself, never in a
directory. Each device publishes the mailbox it receives at inside its MLS
leaf node.

## Mailbox extension

| Field | Value |
|---|---|
| Extension type | `0xF5A1` (MLS private-use range) |
| Carried in | the leaf node of every `KeyPackage` and of every leaf a device creates |
| `extension_data` | exactly 32 bytes: the device's mailbox id |

Every Zelamar leaf lists in its capabilities:
- the ciphersuite `MLS_128_MLKEM768X25519_AES256GCM_SHA384_Ed25519`;
- extension type `0xF5A1`;
- credential type `basic`.

A mailbox id is 32 random bytes. It is not derived from any key, so it can
rotate later through an MLS update of the leaf.

## Verifying members

A leaf is accepted only when all of the following hold:
1. its device certificate verifies (`identity.md`);
2. that certificate certifies the leaf's signature key;
3. it carries a well-formed mailbox extension.

Receivers MUST apply this check to every leaf that enters a chat:
- the key package of an invited device;
- every member, on joining from a welcome;
- in every commit: added members, updated leaves, and the committer's new
  leaf from its update path.

A failing leaf rejects the whole operation, and the device's state stays
unchanged.

## Fan-out

One MLS message goes to many devices.

| Message | Recipients |
|---|---|
| Application message | every member's mailbox except the sender's own |
| Commit adding a member | every member's mailbox before the commit, except the committer's |
| Welcome | the invited device's mailbox only |

The recipients are sorted, without duplicates. Each recipient gets an
envelope (`envelope.md`) with the same envelope id and payload. A message
with no recipients is not sent.
