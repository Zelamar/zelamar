# 0004. Transport-agnostic envelopes (Drift-ready)

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

Zelamar's long-term differentiator is **Drift**: messaging without internet,
over a local network, over Bluetooth or Wi-Fi Direct, or carried
store-and-forward by other Zelamar devices ("bottles"). Retrofitting offline
delivery onto a server-centric protocol would mean a rewrite.

## Decision

- The unit of transmission is a self-contained **envelope**: an opaque,
  routable outer header plus an inner MLS message. The outer header reveals
  neither the sender nor the content. The recipient is addressed by an opaque
  mailbox identifier, designed to become a rotating pseudonym for Drift.
- The core sends and receives envelopes through a **`Transport` abstraction**.
  The MVP ships one implementation (internet via the delivery service). Drift
  adds implementations later without touching chat logic.
- Envelopes are idempotent: they carry a unique id, so duplicates delivered by
  several transports are harmless.
- Timestamps are sender-side UTC milliseconds.
- The envelope format is specified in `protocol/` and versioned
  (`PROTOCOL_VERSION`, currently `0` = pre-release, may change freely).

## Consequences

- Drift becomes "add a transport", not "redesign the protocol".
- Deduplication and ordering live in the core, not the server.
- Some metadata hiding (rotating pseudonymous addresses) must be designed
  carefully in Phase 2 even though only the internet transport exists.
