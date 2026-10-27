# 0019. Calls: WebRTC between two devices, set up inside the encrypted chat

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Calls are the first item of version 2 (CEO decision, 2026-10-26). Nobody
moves from WhatsApp or Telegram to a messenger they cannot call on.

Constraints:
- **End-to-end.** The service, and anyone it uses, must not be able to
  listen in or slip in a device of its own.
- **Budget $0.** No media servers of our own.
- **Windows and Android first,** with iPhone next. One code base, so the
  calls work wherever Flutter runs.
- **No new trust.** The chat already authenticates every device (MLS,
  ADR-0009). Calls should lean on it rather than add a second identity
  system.

## Decision

**WebRTC between the two devices, through `flutter_webrtc`.**
- `flutter_webrtc` 1.6.2+hotfix.4 (MIT, libwebrtc M150, BSD) runs on
  Android, iOS, Windows, macOS and Linux. It is pinned exactly, a version
  three weeks old with the Windows crash fixes.
- Media is DTLS-SRTP from one device to the other: Opus for voice, and
  video when asked for. No media server sits in the middle.

**The chat carries the setup.**
- Three new content kinds go through MLS like text (`protocol/calls.md`):
  - `call` carries the offer;
  - `call-answer` carries the answer;
  - `call-end` says why the call ended.
- The offer and answer hold the DTLS fingerprints. They are signed and
  encrypted by the devices' MLS leaves, so only the two real devices can
  complete the handshake. The service cannot be a man in the middle.
  Insertable streams or SFrame would add nothing for two people.
- Candidates are gathered before sending: one offer, one answer, no
  stream of candidate messages.
- Ringing, "it rings on their side" (the delivered receipt), "answered on
  another device" and "missed" all follow from these messages and the
  history the core already keeps.

**Connectivity from Cloudflare.**
- Its STUN server is free and keyless: most calls connect directly.
- Its TURN relay helps the rest: strict corporate networks, and some mobile
  carriers. It is free for the first 1 000 GB a month, which covers a beta
  many times over, since an hour of voice is about 20 MB. Calls through
  the relay are still end-to-end encrypted.
- The relay needs a TURN key created in the CEO's Cloudflare account and
  stored as two secrets. Until then, the service hands out STUN only.
- `POST /v0/mailboxes/{id}/ice` hands each device its servers. It asks
  for the mailbox secret, and allows 200 lists a day per mailbox, so
  relay credentials cannot be farmed.

**History keeps how a call went, never how it was set up.**
- A call record holds:
  - video or not;
  - who picked up;
  - how it ended;
  - how long it lasted.
- A missed call counts as unread. An answered one does not.
- Calls are not edited, deleted or reacted to.

**Scope of this first version.**
- One-to-one voice and video, in chats the user has accepted.
- Group calls need a media server and wait.
- On Android, a call reaches a phone whose app is closed only through the
  ordinary push. Ringing a closed app with a full-screen call needs a
  foreground service and comes next.

## Consequences

- Calls work without any new server or cost. The relay becomes the first
  usage-priced service, metered by Cloudflare and capped per device by us.
- The other person learns a device's network address, as in any direct
  call. That is why calls only ring in accepted chats. A later "always
  relay" setting can hide it once the relay is on.
- Old versions show an incoming call as "update Zelamar to see this
  message". The beta updates quickly.
- `flutter_webrtc` adds about 20 MB of libwebrtc to each build.
