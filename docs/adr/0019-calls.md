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
- A phone whose app is closed rings too. The offer is marked urgent; the
  service sends a call wake-up with no words; the app starts without a
  window, fetches the offer and rings only for a real call (see "Ringing a
  closed phone" below).

**Ringing a closed phone (Android).**
- The service cannot tell who sends what, so it cannot decide whether a
  phone should ring. If a wake-up rang by itself, anyone who knows a
  username could ring that person's phone over and over.
- So a call wake-up only wakes Zelamar. The process has one Flutter engine
  and with it one session over the store. The wake-up starts the engine
  without a window, or wakes the running one. The window attaches to the
  same engine when it opens. Two sessions over one store never exist.
- The app then fetches its mail like any other start. Only an offer that
  the rules allow to ring makes the phone ring: the system ringtone, a
  full-screen call over the lock screen, Accept and Decline.
- The window shows over the lock screen only while a call rings or goes on.
- A foreground service of type microphone (and camera) runs while a call
  goes on.

## Consequences

- Calls work without any new server or cost. The relay becomes the first
  usage-priced service, metered by Cloudflare and capped per device by us.
- The other person learns a device's network address, as in any direct
  call. That is why calls only ring in accepted chats. A later "always
  relay" setting can hide it once the relay is on.
- Old versions show an incoming call as "update Zelamar to see this
  message". The beta updates quickly.
- `flutter_webrtc` adds about 20 MB of libwebrtc to each build.
- The engine now outlives the window: a closed window still hears calls
  and messages while Android keeps the process.
