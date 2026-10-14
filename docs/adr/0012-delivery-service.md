# 0012. Delivery service: Durable Objects, sans-IO core, anonymous tokens, ordered commits

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

Phase 3 builds the server that "delivers everything and knows nothing".
The constraints:

- **Budget.** $0, on the Cloudflare Workers Free plan. As of October 2026
  the plan gives:
  - 100k requests per day;
  - 10 ms CPU per Worker request, 30 s per Durable Object request;
  - SQLite-backed Durable Objects only, with 5 GB of storage;
  - 100k rows written and 5M rows read per day;
  - WebSocket Hibernation.
- **Privacy.** The server must not see:
  - plaintext;
  - who sent a message;
  - contact lists or stored group membership.
- **MLS ordering.** MLS breaks if two members commit concurrently and each
  merges their own commit: the chat forks forever. The core currently
  merges its own commits at once (ADR-0011). That is safe only while one
  device commits alone.
- **Abuse.** Spam and abuse must be limited without knowing the sender.
- **Network code.** Rust networking would bring an async runtime, TLS
  stacks and platform-proxy handling into every client build. Flutter
  already does networking well on each platform.

## Decision

**One Worker, three kinds of Durable Object (SQLite).**

| Object | Keyed by | Holds |
|---|---|---|
| `Account` | normalized `@username` | the claim, the account public key, device certificates, one-time and last-resort key packages per device |
| `Mailbox` | mailbox id | queued envelopes until acknowledged; live delivery over a hibernating WebSocket |
| `Group` | MLS group id | only the current epoch: the sequencer for commits |

- The object id gives each username, mailbox and group exactly one
  consistent home, so uniqueness and ordering need no locks.
- Search is exact lookup only. Nothing lists usernames.

**Authentication.**
- Account and device operations (claim a username, publish key packages,
  register a mailbox) are signed by the account or device key. Each
  request carries a timestamp and a nonce against replay.
- A mailbox is read with a mailbox secret. The server stores only its hash.
  A mailbox id alone, which is published in the MLS tree and in key
  packages, grants nothing but the right to deposit.

**Sealed sender through anonymous tokens.**
- Depositing an envelope and looking up a username each spend one
  anonymous token.
- Tokens are VOPRF tokens (RFC 9497, Privacy Pass style), issued in
  batches to authenticated devices. Issuance is limited per account, and
  new accounts get smaller allowances.
- The server cannot link a spent token to the account it was issued to. It
  knows that a legitimate user deposited, never which one.
- Spent tokens are remembered until the issuing key rotates.
- Libraries: Rust `voprf` (MIT/Apache-2.0) and `@cloudflare/voprf-ts`
  (BSD-3-Clause). Both sides are tested against the RFC test vectors.

**Registration.**
- A closed-beta invite code.
- A light proof of work, a few seconds on a phone, so no CAPTCHA.
- A reserved-name list.

**Ordered commits.**
- A commit goes to the chat's `Group` object together with its epoch and,
  for an invitation, its welcome.
- The first commit for the current epoch wins: the object advances the
  epoch and fans the commit and welcome out to the mailboxes. Any other
  commit for that epoch is rejected.
- The core changes to match:
  - a commit stays pending until the server accepts it;
  - on rejection the device drops its commit, applies the winning one, and
    retries if still needed.
- Application messages go straight to mailboxes. Receivers keep a few
  past epochs (for late messages) and buffer messages from an epoch they
  have not reached yet.
- **Timing.** The sequencer ships together with the first commit that can
  race: linking a second device, or group chats. Until then the only commit
  in a chat is its creator's first invitation, made before anyone else is
  in the chat. It cannot race, so it is merged at once (ADR-0011).

**Sans-IO core.**
- The Rust core builds, signs and parses every server request and
  response: tokens, proofs of work, envelopes and acknowledgements.
- Dart only moves bytes over HTTPS and WebSocket.
- All security-relevant logic is still written once, in Rust. The core
  stays free of async runtimes and TLS, and Drift transports plug in the
  same way.

**Later, not in v1:**
- empty FCM push notifications (needs a Firebase project);
- R2 attachments (Phase 5);
- per-recipient sealing of envelopes. This hides the group id and the
  identical-ciphertext fan-out from the server.

## Consequences

- **Free quotas.** A message costs:
  - 1 Worker request;
  - 1 `Group` or token check;
  - one request per recipient mailbox;
  - about 2 rows per recipient (write and delete).

  The beta target, 50 users × 200 messages a day, fits with a wide margin.
  A load test will prove it (Phase 3 gate).
- **What the server keeps:**
  - usernames with account keys and device certificates;
  - key packages;
  - hashed mailbox secrets;
  - the current epoch of each group id;
  - spent tokens;
  - envelopes in flight.

  It never keeps plaintext, senders, contact lists or member lists. In
  flight, it sees the routing metadata described in ADR-0011.
- Ordering commits through the server costs one extra hop per membership
  change. Concurrent changes can no longer fork a chat.
- The core grows a small protocol layer (request and response types); the
  app grows a thin transport. Both are testable without a network.
- Tokens add a step before sending. An app that runs out of tokens must
  wait for its allowance: by design, that is the spam limit.
