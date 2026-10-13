# Message content — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/content.rs`.

The plaintext of every MLS application message is one CBOR map:

| Key | Type | Meaning |
|---|---|---|
| `v` | uint | content version, `0` |
| `i` | bytes(16) | message id chosen by the author |
| `t` | uint | sent at: UTC milliseconds since the Unix epoch, author's clock |
| `b` | map | body, externally tagged by kind |

Body kinds in version 0:

| Kind | Fields |
|---|---|
| `text` | `text`: UTF-8 string, at most 32 768 bytes |

Planned kinds (later slices): `reply`, `edit`, `reaction`, `delete`,
`receipt`.

## Rules

1. The encoded content is at most 65 536 bytes. Larger content is rejected
   before decoding.
2. A receiver MUST reject a version above the one it implements, and report
   it instead of misreading it.
3. Unknown map keys are ignored, which leaves room for evolution.
4. Display time is always `t` converted to the viewer's time zone, never
   the receive time. (The prototype got this wrong.)
