# 0001. Record architecture decisions

- **Status:** Accepted
- **Date:** 2026-10-13

## Context

Zelamar is built by a non-technical CEO and an AI CTO working in separate
sessions. Context that lives only in a conversation is lost when the session
ends; the previous prototype even lost its server code entirely. Future human
engineers will also need to know *why* the system looks the way it does.

## Decision

Record every significant technical decision as a short Markdown ADR in
`docs/adr/`, numbered sequentially. `ROADMAP.md` stays the source of truth for
*what* we build and in which order; ADRs hold the *why*.

## Consequences

- Decisions survive across sessions and people.
- Revisiting a decision means writing a superseding ADR, which forces us to
  state what changed.
- Small overhead per decision, paid once.
