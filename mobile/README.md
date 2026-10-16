# Buzz Mobile

Flutter mobile client for Buzz.

See [VISION_MOBILE.md](../VISION_MOBILE.md) for intended behavior and
architecture.

## Optional Builderlab login

`BUZZ_AUTH_DOMAINS` is a comma-separated build-time environment setting, passed
with Flutter's `--dart-define` (shell environment variables alone are not read):

```bash
flutter run --dart-define=BUZZ_AUTH_DOMAINS=communities.buzz.xyz,communities.public.builderlabstaging.xyz
```

Unset or empty means **Builderlab features are disabled**: no account login,
community creation, backup/recovery UI, reconnect prompts, or background account
restoration. QR/code pairing, community links and invites use Nostr-only entry.
Saved account keys and sessions are retained for a later enabled build.

A nonempty list enables Builderlab features and requires account setup only for
matching domains. Those features need their backend endpoints. Normal relay key
authentication remains in place. `BUZZ_BUILDERLAB_URL` selects the backend; setting
it alone does not enable Builderlab features.

Entries match the domain itself and its subdomains, case-insensitively. Use DNS
names, not URLs or wildcards. The list does not add trusted account servers:
configured domains outside the supported Builderlab realm cannot use hosted
setup. `BUZZ_BUILDERLAB_URL` still selects the trusted account environment.

## Hosted relay authentication

`--dart-define=BUZZ_HOSTED_RELAY_AUTH=true` enables Builderlab relay assertions
and reconnect sign-in **only for `BUZZ_AUTH_DOMAINS` matches**. It defaults off: configure backend assertion issuance
before enabling mobile authentication, and enable relay enforcement last.

## Setup

Use the Flutter SDK pinned by the repository. Activate Hermit from the repo
root before resolving packages or running any Flutter command:

```bash
cd /path/to/buzz
. ./bin/activate-hermit
./bin/just mobile-install
```

`mobile-build-android` intentionally builds with `--no-pub`. If an IDE or an
external Flutter SDK has touched `mobile/.dart_tool`, rerun `mobile-install`
with the pinned SDK before building so `flutter_test`, `sky_engine`, and the
engine all come from the same Flutter version.

### iOS toolchain

Use Xcode 27.0 for iOS development and build validation on an Apple silicon
Mac running macOS Tahoe 26.6 or later, as required by the
[Xcode 27 release notes](https://developer.apple.com/documentation/xcode-release-notes/xcode-27-release-notes).
On Intel Macs or hosts that cannot meet those requirements, use a compatible
Xcode 26 version, which remains supported. Activate the repository's pinned
Flutter SDK with Hermit as described above.

When multiple Xcode versions are installed, select the intended version for
the command with `DEVELOPER_DIR`, for example:

```bash
DEVELOPER_DIR=/Applications/Xcode-27.0.0.app/Contents/Developer xcodebuild -version
```

Adjust the path to your installation. Keep the compiler and SDK from the same
Xcode installation. The pinned Flutter native-assets tooling can discover the
globally selected SDK even when `DEVELOPER_DIR` selects another compiler, so
use a build machine with a matching default Xcode when validating either
Xcode 26 or Xcode 27.

The native cryptography dependency is pinned upstream C source in
`ios/BuzzPushKit/Sources/CSecp256k1`; it requires no Swift build-plugin approval.
GitHub's mobile CI currently uses the Xcode default of its `macos-latest`
runner. The internal release toolchain is configured separately in
[buzz-releases](https://github.com/squareup/buzz-releases).

## Run

```bash
# From repo root (applies a worktree-isolated debug identity and starts/reuses Simulator):
just mobile-dev

# Direct (uses the app's configured community; apply worktree overrides first):
cd mobile && flutter run --dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example
```

### Worktree-aware debug identity

Debug builds produced from a git worktree get a unique app identifier keyed
to the **worktree directory name**
(`xyz.block.buzz.dev.mobile.<slug>` on iOS,
`xyz.block.buzz.mobile.<slug>` on Android) plus a display-only branch label
in the app name (`Buzz (my-branch)`, or a short SHA when the worktree is
detached). Because the identifier follows the directory rather than the
branch, one worktree keeps exactly one installed app — and its login state —
across branch switches, and builds from multiple worktrees install side by
side, mirroring the desktop dev experience. Release and profile builds
always keep the production identity and name.

`just mobile-dev` and `just mobile-build-android` apply this automatically by
running `scripts/mobile-worktree-overrides.sh`, which writes two gitignored
files:

- `mobile/ios/Flutter/WorktreeOverrides.xcconfig` (included by Debug builds
  only; a developer's `AppOverrides.xcconfig` is included after it, so
  app-specific overrides like a personal `BUNDLE_IDENTIFIER` for device
  signing always win)
- `mobile/android/worktree.properties` (read by the debug build type only)

Android developers can keep a stable local test identity that takes precedence
over the generated worktree values by creating the gitignored
`mobile/android/AppOverrides.properties`:

```properties
appName=Buzz Pairing
applicationIdSuffix=.device_pairing_e2e1
```

These values are consumed by the debug build type only. The standard
`just mobile-build-android` command can still be used; regenerating
`worktree.properties` does not overwrite `AppOverrides.properties`. Release
and profile builds keep the production `Buzz` name and application ID.

For direct Xcode / Android Studio / `flutter run` development, run
`./scripts/mobile-worktree-overrides.sh` from the repo root once per branch
switch to refresh the display label (the install identity never changes);
the persisted files are then picked up by any subsequent build. In the main
checkout the script is a no-op that removes stale override files, restoring
the plain `Buzz` identity. To enable push in direct Xcode builds and Runner tests, supply a
`BUZZ_PUSH_GATEWAY_URL` build setting in the gitignored
`mobile/ios/Flutter/AppOverrides.xcconfig`; the build phase validates and
passes it through as a Flutter Dart define. Since `//` begins an xcconfig
comment, spell the origin as `BUZZ_PUSH_GATEWAY_URL = https:/$()/push.example`.

For an Android debug build that must remain installed alongside other Buzz
worktree builds, set an explicit launcher name and package suffix when invoking
the generator or a recipe that invokes it:

```bash
BUZZ_PUSH_GATEWAY_URL="https://push.example" \
BUZZ_ANDROID_DEBUG_APP_NAME="Buzz Huddles" \
BUZZ_ANDROID_DEBUG_ID_SUFFIX=".huddles_829c" \
./bin/just mobile-build-android
```

This example produces the debug-only package
`xyz.block.buzz.mobile.huddles_829c` with the launcher label `Buzz Huddles`.
The suffix must start with a dot followed by a lowercase letter and may contain
only lowercase letters, digits, and underscores. Release and profile builds
ignore these overrides and retain the production package and name.

To remove leftover worktree-suffixed installs from booted iOS simulators and
connected Android emulators, run `just mobile-clean` (add `--dry-run` via
`./scripts/mobile-worktree-clean.sh --dry-run` to preview). Production
installs are never touched.

### iOS push capability

Every iOS artifact builds and embeds the Notification Service Extension and
native push bridge. Runtime activation is fail-closed and scoped to the current
relay. After authenticated connectivity and a fully valid NIP-11 `buzz-push-v1` push
descriptor, Buzz independently requests display permission and registers with
APNs. Display denial or request failure does not gate the device token, gateway
enrollment, or lease publication, so a later user opt-in can display pushes
without rebuilding transport authority. An absent, malformed, or unreachable
descriptor leaves push inactive without partial enrollment.

Mobile builds without a gateway origin succeed with push unavailable. To enable
push, supply the gateway origin explicitly:

```bash
flutter build ios --dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example
flutter build apk --dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example
```

The iOS and Android build gates validate any supplied define, rejecting empty or
malformed values. Release/profile builds require an HTTPS origin without an
explicit port. An absent define disables permission requests, APNs registration,
gateway enrollment, and lease publication; Settings shows push as unavailable.
No production gateway is selected implicitly. Enrollment
grants and crash-recovery journals are scoped to this origin. Push has not
shipped to existing users, so there is no legacy-state or cross-gateway
migration. Changing gateways requires fresh enrollment; old installations
expire under their original gateway's lease policy. Current-gateway response
loss is still retried from the exact journaled request.

Relay rollout remains an explicit deployment opt-in. Only deployments with
`BUZZ_PUSH_ENABLED=true` advertise the descriptor and process push. See
`docs/push-gateway-deployment.md` for the canonical gateway profile contract,
manual physical-device proof, measurements, and rollback procedure.

For local physical-device development, override the identity and sandbox
environments in the gitignored `mobile/ios/Flutter/AppOverrides.xcconfig`:

```xcconfig
BUNDLE_IDENTIFIER = com.example.buzz.mobile
BUZZ_DEVELOPMENT_TEAM = YOUR_TEAM_ID
BUZZ_IOS_PUSH_ENVIRONMENT = development
BUZZ_APP_ATTEST_ENVIRONMENT = development
BUZZ_PUSH_GATEWAY_URL = https:/$()/push.example
```

Use your personal bundle ID and team above. Provision both the parent and its
`.NotificationService` extension. Configure an isolated gateway with the matching
App Attest application ID, APNs topic and sandbox certificate. Development
attestation requires the explicit `personal-dev-app-attest` gateway build feature
and `BUZZ_PUSH_APP_ATTEST_ENVIRONMENT=development`; ordinary gateway builds accept
production attestation only. See `docs/push-gateway-deployment.md`.
Validate distributed builds separately using a signed release candidate and
its matching gateway configuration.

Parent app identifiers require Apple's Communication
Notifications capability and a regenerated app provisioning profile. The
Notification Service Extension profile does not require that capability.
Enable it on the personal development App ID for local rich-presentation
validation. Enabling it on distribution App IDs is
a release follow-up and is not performed by this repository change. Without a
matching parent profile, source and unit validation still work, but the app
cannot be signed for a physical device.

APNs and the gateway continue to carry only the constant opaque wake-up. The
extension fetches the message from the scoped relay, verifies message, sender
profile, and channel-metadata signatures, and uses a bounded App Group cache
for names and app-rendered avatar thumbnails. It never fetches an avatar URL;
missing, stale, or invalid enrichment falls back to the verified message with a
short sender pubkey, community subtitle, and no image.

## Checks

```bash
dart format --output=none --set-exit-if-changed .
flutter analyze
flutter test --dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example
```

Or from the repo root: `just mobile-check` and `just mobile-test`.

When writing tests, favor a few journey tests that drive real user flows over
many small logic tests. See
[Testing Conventions](../AGENTS.md#testing-conventions).

## Android release signing

Android release builds fail unless all upload-key inputs are supplied through the
environment:

- `BUZZ_ANDROID_UPLOAD_KEYSTORE_PATH`: path to a CI-vended keystore file
- `BUZZ_ANDROID_UPLOAD_KEYSTORE_PASSWORD`
- `BUZZ_ANDROID_UPLOAD_KEY_ALIAS`
- `BUZZ_ANDROID_UPLOAD_KEY_PASSWORD`

The keystore path must be absolute, and the keystore must remain outside the
repository. Development and debug builds do not require these variables.

Release pipelines that sign through the central APK Signer service instead of
a local upload keystore must set `BUZZ_ANDROID_RELEASE_SIGNING=external`. That
mode produces an unsigned release bundle and refuses to run if any
`BUZZ_ANDROID_UPLOAD_*` value is also set.

## Architecture

```
lib/
├── main.dart              # Entry point, Riverpod bootstrap
├── app.dart               # MaterialApp with theme
├── shared/
│   └── theme/             # Catppuccin light/dark, spacing tokens, extensions
└── features/
    └── home/              # Placeholder home surface
```

- **State management:** Riverpod + Hooks (`HookConsumerWidget`)
- **Theme:** Catppuccin Latte (light) / Macchiato (dark) — matches desktop
- **Spacing:** `Grid` tokens for consistent spacing
- **Linting:** `flutter_lints` + `riverpod_lint` via `custom_lint`
- **Feature isolation:** No cross-feature imports except `shared/`
