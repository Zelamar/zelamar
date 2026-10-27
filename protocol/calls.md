# Calls — version 0

**Status:** draft, normative for protocol version 0. Implemented in
`core/zelamar-core/src/content.rs` and `history.rs` (messages and call
records), `app/rust/src/api/session.rs` (what a call message asks of a
device) and `app/lib/src/calls/` (WebRTC). Decision: ADR-0019.

A call is a WebRTC connection between two devices. Everything needed to set
it up travels inside the chat as ordinary MLS messages, so the service
relays it without reading it, exactly like text.

## Media

- One `RTCPeerConnection` per call: Opus audio, and video when asked for.
- Media is encrypted with DTLS-SRTP between the two devices. The offer and
  the answer carry each side's DTLS fingerprint. They travel inside the MLS
  chat, signed and encrypted by the devices' leaves, so only the device that
  sent the offer and the device that sent the answer can complete the
  handshake. The service, a STUN server or a TURN relay only ever see SRTP.
- Candidates are not trickled. Each side gathers them, for at most 3
  seconds, and sends one complete session description. A call therefore
  takes exactly one offer and one answer.

## Servers

`POST /v0/mailboxes/{mailbox}/ice` with `Authorization: Bearer <secret>`
(the mailbox secret, as for push in `delivery.md`) answers `200`:

```json
{ "ice_servers": [ { "urls": ["stun:stun.cloudflare.com:3478"] },
                   { "urls": ["turn:…", "turns:…"], "username": "…", "credential": "…" } ] }
```

- Cloudflare's STUN server is always listed.
- When the service holds a TURN key, a relay entry follows, with
  credentials that work for 4 hours. URLs on port 53 are left out.
- Each mailbox gets 200 lists per UTC day (`429 over_quota` after that),
  and each network address 30 a minute (`429 rate_limited`).
- Clients accept at most 8 servers of at most 8 `stun:`, `turn:` or
  `turns:` URLs each.

## Waking a phone that is away

The device that calls sends its `call` with `Zelamar-Wake: call`
(`delivery.md`). A device of the called person that has no connection
open gets a call wake-up within seconds:
- **Android:** the wake-up starts Zelamar without a window, or wakes the
  running one. It fetches its mail and opens the offer. If rule 2 below
  says to ring, the phone rings with the system ringtone, over the lock
  screen where Android allows it, with Accept and Decline.
- **Windows:** Zelamar runs in the tray and keeps its connection. A call
  brings the window to the front.

While a call goes on, Android keeps a foreground service ("Call with
@bob"), so the microphone and camera keep working with the screen off.

## Messages

Three content kinds (`content.md`):

| `k` | Fields of `c` | Meaning |
|---|---|---|
| `call` | `video`: bool, `sdp`: string (1–16 384 bytes) | Starts a call. The message id `i` is the call's id |
| `call-answer` | `call`: bytes(16), `sdp`: string (1–16 384 bytes) | One of the called person's devices picks up |
| `call-end` | `call`: bytes(16), `reason`: string, `duration_ms`: uint (optional) | The call ends, or is turned down |

`reason` is one of:
- `hangup`: someone hung up; before an answer, the caller gave up;
- `declined`: the person called said no;
- `missed`: nobody answered in time;
- `busy`: the person called is in another call;
- `failed`: the connection could not be made, or was lost.

An unknown reason is read as "ended" and never sent. `duration_ms` is how
long the two people talked, measured by whoever ends the call from the
moment media connected.

## Rules

1. **Only between two people.** A `call` is sent only in a chat with
   exactly one other account.
2. **Who rings.** A device rings for a `call` only when all of these hold:
   - it comes from the other account;
   - the user has accepted the chat on this device;
   - history has neither an answer nor an end for it;
   - its `t` is at most 60 seconds old by this device's clock (a `t` in
     the future rings).

   A device already in a call does not ring.
3. **Ringing.** The called device's `delivered` receipt for the `call`
   tells the caller that it is ringing.
4. **No answer.** The caller gives up after 45 seconds with `call-end`
   `missed`. A ringing device also stops on its own 60 seconds after the
   `call` arrived.
5. **Picking up.** The device that picks up sends `call-answer`. History
   keeps the first answer from an account other than the caller's. When the
   called person's other devices see it, they stop ringing. A device that
   picked up but sees another device of its account answer first leaves
   without sending anything.
6. **Declining.** `call-end` `declined` stops every device of the person
   called.
7. **Calling each other at once.** When a device that is calling someone
   gets a `call` from them in the same chat, the call with the smaller id
   (bytewise) wins on both sides. The caller of the other call withdraws it
   with `call-end` `hangup` and picks up the winner at once: both people
   wanted to talk.
8. **Ending.** Either person ends a call with `call-end` `hangup` and its
   `duration_ms`. A connection not made within 30 seconds of the answer,
   or lost for 15 seconds, ends with `failed`.
9. **History.** A `call` adds a call record to history: whether it started
   with video, which device picked up, how it ended and how long it lasted.
   The session descriptions are never stored.
   - The first `call-end` wins.
   - Once someone has picked up, only the caller's or the answerer's
     account can end the call.
   - A call nobody here picked up counts as unread. A call someone picked
     up does not.
   - Calls are not edited, deleted or reacted to.
10. **Notifications.** When an incoming call ends before anyone on this
    account picked up, the device notifies "Missed call", unless the chat
    is a request or the reason is `busy`.

## What others learn

- **The service** sees envelopes, as for any message. The `ice` request
  and the `Zelamar-Wake` header tell it that a device is about to call or
  answer, and when. Google sees that a phone got a wake-up, as for any
  message.
- **Cloudflare's STUN and TURN servers** see the devices' network
  addresses and how much encrypted media flows, never its content.
- **The other person** learns this device's network addresses from its
  candidates, as with any direct connection. Calls are only possible in
  chats the user has accepted.
