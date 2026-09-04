# Buzz Mobile

Flutter mobile client for Buzz.

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

## Run

```bash
# From repo root (applies a worktree-isolated debug identity and starts/reuses Simulator):
just mobile-dev

# Direct (uses the app's configured community; apply worktree overrides first):
cd mobile && flutter run
```

### Worktree-aware debug identity

Debug builds produced from a git worktree get a unique app identifier keyed
to the **worktree directory name**
(`xyz.block.buzz.dogfood.mobile.<slug>` on iOS,
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
the plain `Buzz` identity.

For an Android debug build that must remain installed alongside other Buzz
worktree builds, set an explicit launcher name and package suffix when invoking
the generator or a recipe that invokes it:

```bash
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
relay. After authenticated connectivity and a fully valid NIP-11 `nip-pl` push
descriptor, Buzz independently requests display permission and registers with
APNs. Display denial or request failure does not gate the device token, gateway
enrollment, or lease publication, so a later user opt-in can display pushes
without rebuilding transport authority. An absent, malformed, or unreachable
descriptor leaves push inactive without partial enrollment.

Relay rollout remains an explicit deployment opt-in. Only deployments with
`BUZZ_PUSH_ENABLED=true` advertise the descriptor and process push. See
`docs/push-gateway-deployment.md` for the canonical gateway profile contract,
manual physical-device proof, measurements, and rollback procedure.

For local physical-device development, override the identity and sandbox
environments in the gitignored `mobile/ios/Flutter/AppOverrides.xcconfig`:

```xcconfig
BUNDLE_IDENTIFIER = xyz.block.buzz.mobile
BUZZ_DEVELOPMENT_TEAM = EYF346PHUG
BUZZ_IOS_PUSH_ENVIRONMENT = development
BUZZ_APP_ATTEST_ENVIRONMENT = development
```

This exercises the client, extension, relay, and gateway integration without
requiring a dogfood development signing identity. It uses the canonical
gateway's server-owned App Store profile configured for sandbox in the local
development gateway; it does not validate the internally distributed dogfood
artifact or enable the App Store profile in production. Validate dogfood APNs
end to end by cutting an internal release, waiting for it to reach Mobile
Releases/Comp Portal, and installing that signed artifact on a physical device.

Parent app identifiers require Apple's Communication
Notifications capability and a regenerated app provisioning profile. The
Notification Service Extension profile does not require that capability.
Enable it on the personal development App ID for local rich-presentation
validation. Enabling it on the Block dogfood and eventual App Store App IDs is
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
flutter test
```

Or from the repo root: `just mobile-check` and `just mobile-test`.

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
