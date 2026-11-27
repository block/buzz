---
title: "Android push notifications"
tags: [mobile, android, push-notifications]
status: draft
created: 2026-11-15
---

# Android push notifications

## Problem Statement

Buzz on iOS shows a mobile push notification when someone sends you a direct message or mentions you, even when the app isn't running. On Android, nothing arrives until you open the app, so Android users miss messages that iOS users see right away.

## Solution

Android installations get the same mobile push notifications as iOS: the same triggers, real message text, and tapping a notification opens the conversation. The design keeps today's privacy model. The push gateway sends a contentless wake signal, and the app fetches the message from its relay before showing it. The relay can't tell an Android installation from an iOS one.

## User Stories

1. As an Android user, I want a notification when someone sends me a direct message, so that I can reply without keeping the app open.
2. As an Android user, I want a notification when someone mentions me, so that I don't miss messages that need me.
3. As an Android user, I want the notification to show the sender and the message's own text, so that I can decide whether to open it.
4. As an Android user, I want notification text to follow the same notification body rules as iOS, so that code, links and line breaks read the same on both platforms.
5. As an Android user, I want tapping a notification to open that conversation, so that I can respond straight away.
6. As an Android user, I want notifications from one conversation grouped together, so that a busy DM doesn't flood my notification shade.
7. As an Android user, I want separate "Direct messages" and "Mentions" categories in system settings, so that I can silence one without the other.
8. As an Android user, I want Buzz to ask for notification permission at the same point the iOS app asks, so that the prompt arrives when its purpose is clear.
9. As an Android user who declined notification permission, I want the app to keep working normally, so that declining has no other cost.
10. As an Android user, I want a notification even when the app's fetch is slow or fails, so that a message is never silently missed.
11. As an Android user, I want the real notification to appear directly, with a generic one shown only when the fetch is slow and then replaced, so that one message never alerts twice.
12. As an Android user using the app, I want no system notification for a message while the app is open, so that the app doesn't notify me about what I'm looking at.
13. As an Android user whose phone was offline, I want a notification for each message I missed, grouped per conversation, so that coming back online doesn't hide all but the newest.
14. As an Android user, I want no notification for messages I sent myself from another device, so that my own activity doesn't buzz my phone.
15. As an Android user, I want muted channels to stay silent, so that muting means the same thing on every platform.
16. As an Android user, I want no notifications from hellthreads (messages tagging very many people), so that broadcast noise doesn't reach me, as on iOS.
17. As an Android user, I want a message I already saw in the app not to notify me again, so that notifications only show new things.
18. As an Android user signed into several communities, I want notifications from each of them, so that I don't need to switch communities to stay up to date.
19. As an Android user who signs out of a community, I want its notifications to stop, so that a signed-out account can't reach my phone.
20. As an Android user who reinstalls the app, I want notifications to resume after I sign in again, with no leftover state from the old install.
21. As an Android user whose push token rotates, I want notifications to keep arriving, so that token rotation is invisible to me.
22. As an Android user on a custom OS with Google Play services whose boot key the gateway operator has listed, such as GrapheneOS, I want push to work, so that a privacy-focused phone isn't excluded.
23. As an Android user whose phone can't provide hardware-backed attestation, or whose bootloader is unlocked, or that runs a custom OS whose boot key isn't listed, I want the app to work normally without push, and to see that push is unavailable, so that I understand why nothing arrives.
24. As a Buzz member, I want my relay unable to tell which platform each of my devices runs, so that platform details can't help correlate my identities.
25. As a Buzz member, I want push wakes to carry nothing about my messages, so that Google and Apple learn nothing beyond the fact that a wake happened.
26. As a relay operator, I want one push gateway per app distribution to serve both platforms, so that I configure push once per app.
27. As a push gateway operator, I want enrollment to require hardware attestation, so that scripted clients can't abuse the public enrollment endpoints.
28. As a push gateway operator, I want each installation's signed calls rate limited per attested key, and the enrollment challenge quota sized for both platforms, so that one installation can't flood the gateway and an Android launch can't starve iOS enrollment.
29. As a push gateway operator, I want the gateway to refresh Google's attestation revocation list itself, so that revoked keys stop enrolling without manual work.
30. As a distributor of a fork, I want to run my own gateway with my own Firebase project and Android signing key, so that my app's push is independent of anyone else's.
31. As a distributor, I want Firebase configuration supplied at build and deploy time, so that no distributor-specific values live in the open-source code.
32. As an iOS user, I want my notifications to keep working through the rollout, so that adding Android doesn't break iOS.
33. As a developer, I want Android notification building to reuse the app's Dart relay, sign-in and formatting code, so that there is one implementation to keep correct.

## Implementation Decisions

Decisions recorded as ADRs: [0004](../docs/adr/0004-push-gateway-hides-device-platform.md) (the push gateway hides each installation's platform from the relay), [0005](../docs/adr/0005-android-push-enrollment-attestation.md) (Android enrollment attestation) and [0006](../docs/adr/0006-android-notifications-run-app-dart-code.md) (notifications built by the app's Dart code).

**Push gateway**

- One gateway serves one app distribution on both platforms. Its configuration names the iOS app and APNs credentials as today, plus the Android package name, its accepted signing-certificate digests, and the Firebase project and service-account credentials for FCM HTTP v1.
- An installation records its platform and push endpoint. The platform lives only in gateway state reached through the sealed grant; nothing the relay sees reveals it. Existing installations are iOS. Android endpoints are FCM registration tokens, which are not hex, so endpoint validation and size limits become per platform.
- Enrollment takes a platform-specific attestation. iOS keeps App Attest. Android sends a Key Attestation certificate chain for a fresh key generated after the push token and gateway challenge are known. As App Attest's client data does on iOS, the attestation challenge binds the whole enrollment transcript: it is the SHA-256 of the transcript bytes, which contain the gateway challenge, so the push token, epoch and expiry are attested too, and the existing exact-replay recovery rule works unchanged. The gateway accepts the chain only when:
  - it reaches a configured Google attestation root (Google publishes more than one; the list comes from Google's current documentation, and a root change is a reviewed configuration rollout, as for App Attest) and no certificate in it is revoked;
  - the attestation challenge matches the transcript;
  - the root of trust and key properties, read only from the hardware-enforced list (values in the software-enforced list are attacker-controlled), report a locked bootloader and either verified boot state Verified, or SelfSigned with a verified boot key equal to one of the gateway's configured boot-key digests (Unverified, Failed, and SelfSigned with any other key are rejected);
  - both the attestation and KeyMint security levels are TrustedEnvironment or StrongBox, the key origin is generated, and the key is EC P-256 with a sign-only purpose;
  - the attested application id names exactly one package, matching the configured package and a configured signing digest. This is the one value read from the software-enforced list, because the operating system, not the secure hardware, fills it in. It is trusted only because the boot state and boot key vouch for the operating system that wrote it.
- The boot-key list is one generic deployment setting, empty by default, so a default gateway accepts stock Android only. No operating system is named in code. An operator who wants GrapheneOS adds its published per-device verified boot key fingerprints, and updates the list when new phone models ship.
- The gateway fetches Google's revocation list periodically. A stale list fails closed only after a generous grace period.
- After enrollment, every installation call (delegate, revoke a delegation, rotate the endpoint, revoke the installation) is signed, as on iOS. iOS signs with an App Attest assertion. Android signs the same transcript bytes with the attested Keystore key, an ECDSA signature the gateway checks against the public key stored at enrollment. Keystore keys have no signature counter, so replay protection for Android rests on the single-use challenge already in each transcript.
- The enrollment request carries no platform field. It holds either an App Attest object or an Android attestation chain, and the gateway derives the platform from which one is present.
- Signed installation calls are rate limited per attested key. Attestation carries no stable device id and each enrollment makes a fresh key, so enrollment itself stays bounded by the deployment-wide challenge quota and by one installation per push token. That quota is sized for both platforms, and the Android client jitters its first enrollment so a release doesn't make every phone enroll at once.
- Delivery moves to one platform-neutral route. The existing APNs-named route stays as an alias until relays have moved over. The gateway maps the lease's priority class to each platform.
- For Android, the wake signal is a high-priority, data-only FCM message whose payload is fixed and compiled in, like today's APNs payload. It contains no relay-supplied bytes.
- Android wakes carry one constant collapse key and a time to live taken from the delivery's expiry. A collapse key only replaces a wake FCM is still holding for an unreachable phone, so an online phone still gets one wake per message, and a phone coming back online gets one wake instead of a burst (FCM keeps at most 100 uncollapsed messages per device and discards them all past that). The app treats FCM's deleted-messages callback as a wake.
- One delegation per installation and relay stays the rule for now (ADR 0004 keeps per-identity grants possible later).

**Relay**

- The lease keeps its `transport` member, and every platform sends the same fixed value that iOS sends today. It is a protocol constant, not device data, so it reveals nothing about the platform. Keeping it lets new clients, Android included, work with relays already deployed, and today's iOS clients work with new relays, so nothing has to roll out in a set order.
- The delivery URL setting points at the gateway's platform-neutral route.
- Classes stay the single default class.

**Mobile, Android native**

- A Firebase messaging service receives the wake. When the app is in the foreground (a native lifecycle check), it does nothing: the running app already shows new messages. Otherwise it starts a background Flutter engine running the wake-to-notification entrypoint and posts the result directly. The category is known only after the fetch, so no notification is posted before it. If nothing has been posted after about 2 s, it posts a generic "New activity" notification in a third "Other" category, which the real result replaces silently, and which is removed when the result is that there is nothing to show. When the result reports a message it claimed but could not return, the generic notification stays, beside the named notifications, and later wakes leave it. A native watchdog finishes the work inside the platform's time limit even if Dart never answers.
- FCM token registration and rotation feed the same enrollment and endpoint-rotation flow iOS uses.
- The app generates an Android Keystore key with the gateway's challenge and sends its attestation chain at enrollment.
- Notification categories are "Direct messages", "Mentions" and "Other" (only the generic notification, for a slow fetch or a message claimed but not returned). Notifications use the platform's messaging style, grouped per conversation. A tap opens the conversation through the app's existing deep-link handling.
- The Android 13+ notification permission prompt appears at the same point iOS asks.
- Firebase configuration is a build-time setting supplied by each distributor, never committed.

**Mobile, shared Dart**

- One wake-to-notification function does what the iOS extension's resolver does. It reads the configured communities, signs in to each relay, fetches events with an overlap rather than strictly newer than the last one shown (a new message can carry an earlier `created_at` because of clock skew or delivery order, and the relay accepts timestamps up to 15 minutes off), deduplicates by event id against the shown record within that reorder window, as the iOS resolver does, applies the same eligibility rules (self-authored, muted, hellthread), and returns every eligible unshown message, bounded (newest 20 overall, with a per-conversation cap that fits the messaging style), each with title, body, conversation and navigation target. The Android handler posts or updates one notification per conversation, each alerting once. A wake therefore means "sync now", so a lost wake is recovered by the next one. iOS keeps one notification per push, because its extension can only rewrite the push it was handed. It runs headless, with no UI and no Riverpod app state, and uses short network timeouts of its own.
- The notification body formatter is ported to Dart and checked against the shared notification-body fixtures, as ADR 0002 requires of every platform.
- The background path records which messages it has shown, so the app and later wakes don't notify for them again. A process runs one background fetch at a time: a wake that arrives mid-run sets a rerun flag instead of starting a second engine. The shown record is updated in one atomic step together with the decision to post, in a store a headless isolate can read, since the foreground app writes the same record.
- Enrollment gains the Android attestation, and every later gateway call is signed with the attested key. The rest of the push layer (subscriptions, renewal, revocation outbox, setup recovery) is shared with iOS unchanged.

Prototype evidence ([`prototype/android-push-headless-dart`](https://github.com/block/buzz/tree/prototype/android-push-headless-dart)): on an emulator, the real text was posted about 0.1 s after a cold wake, and about 0.8 s median (worst about 1 s) with the CPU slowed about 8x. Memory was about 57 MB PSS. A relay that accepts TCP and never answers held the handler until the watchdog fired. On a Pixel 9a over USB (median of 12 cold runs), the real text was posted 0.51 s after the wake (worst 0.54 s), about 45 MB PSS. Most of that time is native engine and Dart startup; the relay round trips over USB took about 0.16 s. The watchdog ended at 9.8 s on the phone, so the real one should fire at about 8 s. No run crossed a real network: the phone couldn't reach the test relay over Wi-Fi.

## Testing Decisions

Tests check behavior at two points, through public interfaces, with fakes only at external boundaries.

- **Push gateway HTTP API.** Extend the existing HTTP delivery and attestation tests with a fake FCM sender and generated Key Attestation chains: enroll, delegate, deliver, rotate endpoint and revoke for an Android installation, and table rows for each rejected attestation (unlocked bootloader, Unverified or Failed boot state, SelfSigned with an unlisted key, SelfSigned with an empty list, a boot key supplied only in the software-enforced list, wrong package or signing digest, wrong challenge, revoked certificate, software-backed key), rows that accept Verified with any key and SelfSigned with a listed key, plus the per-key rate limit. Signed installation calls get matching rows: wrong key, altered transcript, reused challenge. Prior art: the gateway's HTTP delivery tests and App Attest assertion tests.
- **Dart wake-to-notification function.** Run it against a fake relay as a journey: DM and mention notify; self-authored, muted and hellthread messages don't; an already shown message doesn't notify again; a message that arrives after a newer one but carries an older timestamp still notifies; several messages across two conversations after one wake; a missed wake recovered by the next one; several communities; slow and unreachable relays return a fallback inside the time budget; two overlapping wakes, and the foreground app writing the shown record during a background run, never notify twice or lose a message. The ported body formatter runs the shared notification-body fixtures. Prior art: the iOS resolver tests in BuzzPushKit.
- **Relay.** The delivery URL setting accepts the platform-neutral route, in the existing relay configuration tests.
- **End to end.** Before readiness, exercise the real flow on a physical Android phone and an iOS device: enroll, receive a DM and a mention with the app killed, tap through, rotate the token, and sign out.

## Out of Scope

- Sender avatars in Android notifications.
- Push suppression based on desktop activity, which will be designed for all platforms together.
- Notifications for replies that don't mention you, or for general channel messages. New Android categories get added when those exist.
- UnifiedPush and phones without Google Play services (see ADR 0004 for why we're interested).
- Per-identity gateway grants.
- Event ids or encrypted content in the push.
- Moving the iOS notification extension onto the Dart code (see ADR 0006 for the memory condition).
- Phones without hardware-backed attestation, with an unlocked bootloader, or running a custom OS whose boot key isn't listed.

## Further Notes

- Real network latency over cellular is the main remaining performance risk; the prototype's relay ran on the same machine.
- Android deprioritizes apps whose high-priority pushes don't show a notification, which is why a slow fetch still ends in the generic notification.
- Each distributor sets up a Firebase project, registers its Android app, creates a service account for the gateway and records its signing certificate digests once. An agent can do most of this with the Firebase and Google Cloud CLIs, using a signed-in browser only for console-only steps.
- Known platform limits we don't control: after a user force-stops the app, FCM delivers nothing until the app is opened again, and some vendors' battery managers delay high-priority data messages.

## Tickets

Work blockers first. Each ticket is a vertical slice that can be verified on its own.

1. **Platform-neutral delivery route.** The gateway accepts deliveries on a platform-neutral route, keeping the APNs route as an alias, and the relay's delivery URL setting accepts it. iOS keeps working. Blocked by: none.
2. **Gateway records installation platform and sends FCM wakes.** Adds the platform (derived from the attestation type) and per-platform endpoints, Keystore-signed installation calls, the FCM HTTP v1 sender (constant collapse key, time to live from the delivery expiry) and the Android configuration. Enrollment is tested end to end with a test-only attestation verifier, using a fake FCM sender. Blocked by: 1.
3. **Gateway verifies Android Key Attestation.** Adds Google roots, the revocation list refresh, the boot-state, boot-key list and app-identity checks, and the per-key rate limit. Tested with the table of rejected attestations. Blocked by: 2.
4. **Dart notification body formatter.** Ports the formatter to Dart, passing the shared notification-body fixtures. Blocked by: none.
5. **Dart wake-to-notification function.** Headless and journey-tested against a fake relay, returning every eligible unshown message, including the already-shown record, several messages after one wake, overlapping wakes and the fallback inside the time budget. Blocked by: 4.
6. **Android wake handling.** Firebase messaging service, the foreground check, background engine, the 2 s generic notification, deleted-messages handling, watchdog, categories, per-conversation grouping and tap-to-open, with wakes triggered locally, starting from the [prototype](https://github.com/block/buzz/tree/prototype/android-push-headless-dart). Verified on a physical phone. Blocked by: 5.
7. **Android enrollment and lease publication.** FCM token, Keystore attestation, enrollment, signed installation calls, token rotation, the permission prompt and sign-out revocation, wired into the shared push layer. End to end on a physical phone against a test gateway and relay. Blocked by: 3, 6.
