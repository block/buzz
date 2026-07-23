# Buzz Mobile

Flutter mobile client for Buzz.

## Setup

```bash
cd mobile
flutter pub get
```

## Run

```bash
# From repo root (recommended — starts Docker, relay, and simulator):
just mobile-dev

# Direct (requires services and relay already running):
cd mobile && flutter run
```

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
