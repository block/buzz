# App Clip invite handoff (demo)

iOS has no first-party deferred deep link: nothing survives an App Store
install, so a scanned invite QR normally ends in "install the app, then go
back to the web page and tap the link again". This demo shows the native
alternative: an **App Clip as the invite carrier**. The clip launches straight
from the Camera with the invite URL, shows who is inviting you, and on **Join**
parks the invite where the full app can read it, then offers the install. The
full app's first launch replays that invite through the normal deep-link
pipeline — no web page, no second link.

```
QR → Camera → BuzzClip (SwiftUI)             Buzz.app (Flutter)
             parse invite URL                 AppDelegate → BuzzAppClipInviteBridge
             fetch NIP-11 name/icon           PendingDeepLinkNotifier.build()
             Join: BuzzPendingInviteStore ──▶   takePendingAppClipInvite()
                   (clip Keychain + App Group)  → open(uri) → InviteDeepLink
             SKOverlay → install full app       → DeepLinkDispatcher → join sheet
```

## Pieces

| Path | Role |
|------|------|
| `ios/BuzzClip/` | App Clip target (`BuzzClipApp`, `InviteClipModel`, `InviteClipView`, `Info.plist`, entitlements). Pure SwiftUI; no Flutter, so it stays far under the 15 MB physical-invocation cap. |
| `ios/Shared/BuzzInviteLink.swift` | Swift mirror of `parseInviteDeepLink` / `validateInviteRelayUri` (`lib/shared/deeplink/deep_link.dart`, `lib/shared/relay/relay_validation.dart`). Compiled into both Runner and the clip. |
| `ios/Shared/BuzzPendingInviteStore.swift` | The handoff record: an item in the clip's own Keychain (iOS 15.4+ migrates an App Clip's Keychain items into the full app at install; App Clips cannot hold the Keychain Sharing entitlement) plus App Group `UserDefaults` (fallback, and the only shared copy on the simulator). `take()` reads and clears both. |
| `ios/Runner/AppClipInviteBridge.swift` | `buzz/app_clip_invite` method channel; `takePendingInvite` returns the stored URL once. |
| `lib/shared/deeplink/app_clip_invite_bridge.dart` | Dart side of the channel; `appClipInvitePickupProvider` is the test seam. |
| `lib/shared/deeplink/pending_deep_link_provider.dart` | `PendingDeepLinkNotifier.build()` drains the pickup into the existing FIFO, exactly like a cold-start universal link. |
| `ios/Runner.xcodeproj/xcshareddata/xcschemes/BuzzClip.xcscheme` | Scheme with `_XCAppClipURL` so the clip launches with a sample invite on the simulator. |

Entitlements: both targets carry the App Group; the clip adds
`com.apple.developer.parent-application-identifiers` and
`appclips:$(BUZZ_INVITE_LINK_HOST)`, Runner adds
`applinks:$(BUZZ_INVITE_LINK_HOST)`. The clip must **not** carry
`keychain-access-groups` — the system refuses to install an App Clip with it
("not allowed on app clips"); the full app keeps its existing group and reads
the handoff item without pinning one. `BUZZ_INVITE_LINK_HOST` lives in
`ios/Flutter/{Debug,Release}.xcconfig` and is a placeholder (see follow-ups).

## Running the demo (simulator)

Prerequisites: `just mobile-install`, a booted iPhone simulator, and ideally a
local relay (`just relay`, ws://localhost:3000) so the clip has real community
metadata and the app has somewhere to send the claim.

1. **Build and run the clip.** In Xcode (`open ios/Runner.xcworkspace`) pick
   the **BuzzClip** scheme → an iPhone simulator → Run. The scheme's
   `_XCAppClipURL` environment variable
   (`http://localhost:3000/invite/demo-invite-code`) stands in for the QR
   scan; edit it in *Edit Scheme → Run → Arguments* to point at another relay or
   a real invite code. Plaintext `http://localhost` is only accepted in Debug,
   mirroring the Dart parser.

   Expected: the card shows the relay's NIP-11 `name`/`description`/`icon` (from
   `GET http://localhost:3000/` with `Accept: application/nostr+json`), or the
   host and a "Community details unavailable" note if the relay is down. An URL
   that is not an invite shows "Not a Buzz invite".

2. **Tap Join.** The invite is written to the clip's Keychain and the App Group,
   the view flips to "Invite saved", and `SKOverlay.AppClipConfiguration` is
   requested. On the simulator the App Store overlay has nothing to show, so the
   handoff is the observable result, not the overlay.

3. **Launch the full app** (`just mobile-dev`, or the **Runner** scheme). On
   first frame `PendingDeepLinkNotifier` calls `takePendingInvite`, the native
   side returns the URL and clears both copies, and the link is parked as an
   `InviteDeepLink`. `DeepLinkDispatcher` then opens the invite join sheet (or,
   pre-onboarding, routes it through the hosted-app gate) exactly as if the URL
   had arrived as a universal link. With a real invite code from your local
   relay the claim completes and you land in the community; with the sample code
   you see the join sheet fail with "This invite is not valid" — which still
   proves the handoff.

   Headless shortcut (no Simulator GUI to tap Join in): write the record the
   clip would write straight into the app's App Group suite, then launch.
   Launching clears it, and the join sheet appears for `relay.example.com`.

   ```bash
   UDID=booted; APP=xyz.block.buzz.dev.mobile   # use your debug bundle id
   PLIST="$(xcrun simctl get_app_container $UDID $APP group.$APP)/Library/Preferences/group.$APP"
   HEX=$(printf '{"url":"https://relay.example.com/invite/demo-invite-code","savedAt":0}' | xxd -p | tr -d '\n')
   xcrun simctl spawn $UDID defaults write "$PLIST" buzz.app-clip.pending-invite.v1 -data "$HEX"
   xcrun simctl launch $UDID $APP
   ```

4. Relaunch the app: nothing happens — the record is single-use.

Command-line equivalents:

```bash
# Clip only (no CocoaPods needed)
xcodebuild -project ios/Runner.xcodeproj -scheme BuzzClip -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO build

# Full app with the clip embedded (Payload/Buzz.app/AppClips/BuzzClip.app).
# Xcode 27.1's `lipo -verify_arch` rejects Flutter 3.41's two-arch check
# ("does not contain architectures \"arm64 x86_64\""); build arm64-only there.
FLUTTER_XCODE_ARCHS=arm64 FLUTTER_XCODE_ONLY_ACTIVE_ARCH=YES \
  flutter build ios --simulator --debug

# Install both bundles and launch the clip with an invocation URL. The env
# var stands in for the scan (UIKit maps it when Xcode launches the clip; the
# clip's DEBUG builds also read it themselves so this works from simctl).
xcrun simctl install booted build/ios/iphonesimulator/Buzz.app
xcrun simctl install booted build/ios/iphonesimulator/Buzz.app/AppClips/BuzzClip.app
SIMCTL_CHILD__XCAppClipURL='http://localhost:3000/invite/demo-invite-code' \
  xcrun simctl launch booted "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
    build/ios/iphonesimulator/Buzz.app/AppClips/BuzzClip.app/Info.plist)"

# Swift tests for the shared parser/store (RunnerTests host = Buzz.app)
xcodebuild test -workspace ios/Runner.xcworkspace -scheme Runner \
  -destination 'platform=iOS Simulator,name=iPhone 17' \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES \
  -skipPackagePluginValidation -skipMacroValidation \
  -only-testing:RunnerTests/BuzzInviteLinkTests \
  -only-testing:RunnerTests/BuzzPendingInviteStoreTests

# Dart tests for the pickup and the channel contract
flutter test test/shared/deeplink/
```

Toolchain notes (unrelated to the clip, but they block the Runner build):
`xcodebuild` on fresh derived data refuses the `SharedSourcesPlugin` build
plug-in that `BuzzPushKit` pulls in via `swift-secp256k1` until it is trusted
once in Xcode's UI; `-skipPackagePluginValidation -skipMacroValidation` is the
CLI equivalent. Xcode 27 also rejects the sub-15.0 deployment targets CocoaPods
generates for several pods — `main` raises them in the Podfile's
`post_install` (#8379), so a branch without that hunk needs it before
`pod install`.

## Real vs. stubbed

Real:

- URL acceptance rules (HTTPS `/invite/<code>`, `buzz://join`, TLS/localhost
  and public-IP-literal rules) — the same inputs Dart accepts, covered by
  `BuzzInviteLinkTests`.
- Community metadata via the relay's public NIP-11 document — the same
  unauthenticated endpoint `communityIconProvider` already reads. No new API.
- Keychain + App Group handoff store, single-use `take()`, Flutter channel, and
  the hook into `PendingDeepLinkNotifier` (covered by `BuzzPendingInviteStoreTests`
  and `test/shared/deeplink/`).
- SKOverlay request with `AppClipConfiguration`.

Stubbed / demo-only:

- **Invocation.** The scheme's `_XCAppClipURL` replaces the Camera. A real QR
  invocation needs the App Store Connect / AASA work below; the pre-release path
  on a device is *Settings → Developer → Local Experiences*.
- **`BUZZ_INVITE_LINK_HOST = invite.buzz.example`** is a placeholder. The Dart
  parser accepts any host, so there is no single invite domain yet to register.
- **Install.** On the simulator the clip and the full app are simply two
  installed bundles; SKOverlay cannot install anything and no Keychain
  migration happens, so the App Group copy is what the full app finds there.
  On a device, the full app replaces the clip and receives its Keychain items
  (iOS 15.4+).
- The clip does not sign the user in. Per the HIG a shipping clip should do
  something useful (finish sign-in, show the first screen) rather than only
  carry the token; this demo stops at Join to keep the surface small.

## Production follow-ups

1. **Pick the invite domain(s)** and publish an AASA with both `applinks` and
   `appclips` entries for the clip bundle ID; set `BUZZ_INVITE_LINK_HOST`
   accordingly (and extend to multiple hosts if invites stay per-relay).
2. **App Store Connect:** register an *advanced App Clip experience* for the
   URL prefix (required for physical QR/NFC invocations) and give the clip an
   icon, header image, and card copy. Add a default experience as the Safari
   fallback and the `apple-itunes-app` meta tag (`app-clip-bundle-id`,
   `app-clip-display=card`) to the invite landing page.
3. **Provisioning:** the clip bundle ID (`<app>.Clip`) and the parent app both
   need App IDs with App Groups and Associated Domains (the parent additionally
   keeps Keychain Sharing, which App Clips cannot use); the
   parent additionally gets the managed
   `com.apple.developer.associated-appclip-app-identifiers` entitlement via its
   profile. `buzz-releases` must sign the clip alongside Runner. Until then the
   Runner `applinks` entitlement added here will fail device signing on App IDs
   without Associated Domains — simulator builds are unaffected.
4. **Size check:** App Clips invoked by QR must be ≤ 15 MB uncompressed after
   thinning (iOS 16+). Verify with an archive's App Thinning Size Report once
   signing exists; the SwiftUI clip is currently well under 1 MB.
5. **iOS < 15.4 fallback:** Keychain migration is 15.4+; the App Group copy is
   the only path below that. The deployment target is 16.0, so this is moot
   today — if it drops, keep the App Group write and consider hashing anything
   more sensitive than the invite URL.
6. **Full-app parity:** the full app must handle every invocation URL the clip
   does (it already does via `parseInviteDeepLink`). When the app is installed
   the card may still appear but launches the full app with the URL.
7. **Lifetime:** clip data is purged after ~10 days of non-use and the clip
   itself after 30. Fine for an invite flow; do not rely on it for anything
   durable.
