# Mobile instructions

Read the root [`AGENTS.md`](../AGENTS.md) and [`README.md`](README.md) first.
The mobile client uses Flutter, Riverpod, and `flutter_hooks`.

- Use `HookConsumerWidget` or `ConsumerWidget`; do not introduce
  `StatefulWidget`. Use Riverpod for state and hooks for local state.
- Do not run `flutter run`, `flutter build`, `flutter clean`, or `flutter
  upgrade` as an agent. Safe validation commands are `dart format`, `flutter
  analyze`, and `flutter test`.
- Do not use `print()`; use `debugPrint()` or structured logging.
- Keep feature modules isolated: do not import one feature from another; share
  code through `lib/shared/`. Prefer `context.colors`, `context.textTheme`,
  `Grid`, and `Radii` over raw theme values and ad hoc spacing.
- Keep widgets small: one public widget per file, with private pieces in sibling
  parts. The 1,000-line file limit is enforced by `just mobile-check`; split a
  file rather than raising the limit.
- Prefer widget tests. Use `ProviderScope` overrides and fakes that extend the
  real notifier when needed.

Run `just mobile-check` and `just mobile-test` (or the equivalent mobile
commands) for mobile changes.
