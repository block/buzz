## Mobile App (Flutter)

The mobile app lives in `mobile/` — a Flutter app using Riverpod + Hooks.

### Architecture

- **State management:** Riverpod + `flutter_hooks` (`HookConsumerWidget`)
- **Theme:** Catppuccin Latte (light) / Macchiato (dark) — matches desktop
- **Features:** Isolated under `lib/features/`, shared code in `lib/shared/`
- **Nostr models:** `lib/shared/relay/nostr_models.dart` — event kinds must
  stay in sync with `desktop/src/shared/constants/kinds.ts`

### Rules

- **NEVER use `StatefulWidget`** — favor Riverpod for state and always use
  `HookConsumerWidget` or `ConsumerWidget` with `flutter_hooks` for local state.
- Agents may build and run the Flutter app when it materially helps implement,
  debug, or validate mobile changes. Prefer the smallest relevant command and
  reuse an already-running simulator/emulator and the app's configured staging
  or production community when that is sufficient. Do not start or rebuild
  local relay services unless the task specifically requires relay-side or
  isolated integration behavior.
- For iOS runtime validation, prefer `just mobile-dev`; it applies the
  worktree-specific debug identity and runs `flutter run`. Direct `flutter run`
  or IDE workflows are also allowed. Use `just mobile-build-android` only when
  an APK build is relevant to the task.
- Do not rebuild, reinstall, or relaunch merely for ceremony. Preserve Flutter's
  incremental build cache and use hot reload/restart where appropriate. Use
  `flutter clean` only when stale build artifacts are a credible cause. Run
  `flutter upgrade` only when the task explicitly requires a toolchain change.
- For user-visible or integration changes, exercise the affected workflow in a
  real app when practical and report the device/simulator, connected community,
  and workflow actually tested.
- **Do NOT use `print()`** — use `debugPrint()` or structured logging.
- Prefer `context.colors` and `context.textTheme` (via theme extensions)
  over raw `Theme.of(context)` calls.
- **Keep widgets small and composable.** One public widget per file; push
  private sub-widgets (`_Foo`) into sibling `part` files under a
  `<page>/` folder rather than growing the page file. Mobile's hard ceiling is
  **1200 lines/file**, enforced with the other surface-specific limits by the
  repository-level `just file-size-check` gate (`just check`, CI, and every
  pre-push). If an individual file trips the guard, **split the file — never
  bump a surface limit or add an override merely to admit that file.**
  Deliberate repository-wide policy revisions must update the enforced rules,
  tests, and guidance together.
- Feature modules must not import from other feature modules — only from
  `shared/`.
- Use `Grid` tokens for spacing, `Radii` for border radius.

### Quality Checks

```bash
cd mobile
dart format --output=none --set-exit-if-changed .
flutter analyze
flutter test --dart-define=BUZZ_PUSH_GATEWAY_URL=https://push.example
```

Or from repo root: `just mobile-fmt` (auto-fix), `just mobile-check` (lint + fmt check), `just mobile-test` (tests).

To run the app locally with a worktree-specific debug identity and a
started or reused iOS Simulator:

```bash
just mobile-dev
```

This runs `flutter run` against the app's configured community; it does not
start Docker or local relay services.

When run from a git worktree, `just mobile-dev` (and `just
mobile-build-android`) give the debug build a per-worktree app identifier
(keyed to the worktree directory name) and a branch-labelled app name via
`scripts/mobile-worktree-overrides.sh`, so builds from multiple worktrees
install side by side. Release builds are unaffected. `just mobile-clean`
removes stale worktree-suffixed installs from simulators/emulators. See
[mobile/README.md](../../mobile/README.md) for direct Xcode / Android Studio
usage.

### Testing Conventions

- Prefer **widget tests** over unit tests for UI components — test the
  whole widget tree, not individual methods.
- Use `ProviderScope(overrides: [...])` to inject fake notifiers.
- Fake notifiers should extend the real notifier class and override `build()`.
- Use the `WidgetHelpers.testable()` wrapper for simple widget tests or
  build a custom `ProviderScope` + `MaterialApp` when you need specific overrides.

---
