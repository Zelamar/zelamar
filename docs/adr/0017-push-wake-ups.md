# 0017. Push: an empty wake-up through Firebase, sent only when a device is away

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

Android stops background network use soon after an app leaves the screen,
so a closed Zelamar misses messages until it is opened again. Android's
answer is Firebase Cloud Messaging (FCM): Google's servers keep one
connection per phone and wake apps on request.

Constraints:
- **Privacy.** Google must learn nothing about content or senders.
- **Budget.** FCM is free.
- **Windows** needs no push in v1, because Zelamar keeps running in the
  tray:
  - closing the window only hides it;
  - the tray icon's menu opens Zelamar again or quits it;
  - launching a profile that is already running brings its window back.

## Decision

**Push carries nothing but "you have mail".**
- The payload is a fixed, content-free notification: "Zelamar" and "New
  message", on a dedicated channel.
- Android shows it while the app is closed. Opening the app connects the
  mailbox and shows the real messages.
- Google learns only that this phone got some message at that time. The
  service already knew that.

**The device registers its FCM token with its mailbox.**
- `PUT /v0/mailboxes/{id}/push` with `{"platform": "fcm", "token": …}`,
  authorized like reading the mailbox (`Bearer <secret>`).
- `DELETE` on the same path forgets the token, for example when
  notifications are turned off or the device is erased.
- The token lives only in the mailbox object. Nothing ties it to a
  username.

**The mailbox decides when to wake.**
- An envelope arrives and no live connection takes it: the mailbox asks
  for a push.
- At most one push per mailbox every 30 seconds, so a burst of messages
  is one wake-up.
- An invalid token (FCM answers `404` or `UNREGISTERED`) is forgotten.

**One `Push` Durable Object talks to Google.**
- It signs an RS256 JWT with the Firebase service account and exchanges it
  for an access token. It keeps the token for its lifetime, about an hour.
- It sends through the FCM HTTP v1 API.
- The service account is a Cloudflare secret (`FCM_SERVICE_ACCOUNT`).
  Without it, pushes are counted and skipped, which keeps tests and local
  runs offline.

**Only the Android app touches Firebase, and only to get its token.**
- `MainActivity.kt` uses the Firebase Messaging Android SDK (Apache-2.0)
  directly, over the `zelamar/push` channel. The FlutterFire plugins would
  also pull the Firebase C++ SDK into the Windows build.
- The app's public Firebase identifiers come from the build environment
  (repository variables in `release.yml`), as the string resources
  Firebase reads at start-up. Without them the APK has no push and works
  as before.
- Firebase auto-initialization and analytics are off. The token is fetched
  only after an account is ready, and it is sent to the mailbox once per
  token.
- The SDK brings Google Play services, which are proprietary but free to
  ship. This is the one exception to the permissive-licence rule
  (ADR-0005), and it is confined to Android push.
- Windows does not use Firebase at all.

## Consequences

- A closed Android app learns of new messages within seconds, and Google
  sees no content.
- Push depends on Google Play Services. Phones without them, and people
  who refuse Google, get messages when they open the app. A battery-hungry
  foreground connection is a possible later option.
- The Firebase project and its service account belong to the CEO's Google
  account. The service account key is a secret like the invite key: sealed
  on the build machine and stored in Cloudflare.
