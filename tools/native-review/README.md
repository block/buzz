# Buzz native review harness

The native review harness exercises the app shapes that browser tests cannot:
real Tauri/WKWebView or Flutter launch, OS input delivery, native rendering,
recording, app lifecycle, and cleanup. It writes reviewable evidence tied to the
source revision that produced it. It complements `just ci`, package tests, and
Playwright; it does not replace them.

Supported lanes:

- **macOS Desktop:** declarative journeys driven through Accessibility and
  CGEvent, with window-only MP4, screenshots, semantic/AX snapshots, app
  CPU/RSS sampling, and performance comparison.
- **iOS Simulator:** Flutter integration journeys on an erased simulator, with
  full-device MP4, screenshot, Flutter log, simulator provenance, and cleanup
  receipt.

## Safety contract

Desktop runs accept only loopback `ws://`/`http://` relays. Every run gets an
ephemeral Nostr key, run-specific dev bundle ID, keyring service, HOME,
WebKit/app-data state, and artifact directory. The launcher environment is
allowlisted before the ephemeral key is added; inherited tokens and production
keys never enter the reviewed process. Production bundle IDs, keyring services,
and remote relays fail closed.

iOS runs erase the selected simulator before and after the journey. Do not point
the runner at a simulator containing state you need. The review-only environment
suppresses the launch notification prompt because Flutter cannot actuate
SpringBoard; normal app launches retain production permission behavior.

These controls protect reviewer state from accidents. They are **not**
containment for hostile code. Use a dedicated macOS user, disposable simulator,
or VM for untrusted changes.

## One-time setup

1. Activate the repository toolchain and install dependencies:

   ```bash
   . ./bin/activate-hermit
   just setup
   ```

2. For Desktop, start the isolated `buzz-harness` relay on port 3030 as directed
   by `scripts/start-isolated-test-relay.sh`, then grant **Accessibility** and
   **Screen Recording** to the terminal or agent process that launches the run.
   Verify both permissions independently:

   ```bash
   just native-review-doctor
   ```

3. For iOS, install Xcode with an iOS Simulator runtime, then list available
   devices. The exact name is passed to the runner:

   ```bash
   xcrun simctl list devices available
   ```

## Run existing journeys

```bash
# Desktop
just native-review-desktop tools/native-review/desktop/tooltip-fresh-dwell.yaml
just native-review-desktop tools/native-review/desktop/composer-keyboard.yaml
just native-review-desktop tools/native-review/desktop/search-shortcut-dismissal.yaml

# iOS (defaults to iPhone 17 Pro)
just native-review-ios
just native-review-ios 'iPhone 17 Pro'

# Harness tests
python3 -m unittest discover -s tools/native-review/tests -p 'test_*.py'
```

Artifacts are written under:

```text
test-results/native-review/<12-char-sha>/<flow>/<run-id>/
```

A failed locator, postcondition, Flutter test, recording, evidence capture, or
cleanup returns nonzero and leaves a partial failed receipt. A useful report
includes the exact receipt path, full HEAD SHA, dirty status, and whether cleanup
passed. Never describe a dirty receipt as clean-SHA proof.

## Author a Desktop journey

Copy the nearest checked-in journey rather than starting from an empty file.
`desktop/composer-keyboard.yaml` demonstrates fallback locators, text entry,
scrolling, focus/value assertions, and cleanup. The schema is
`schemas/journey.schema.json`.

Every journey declares:

- `flow`: stable artifact/budget identity using lowercase letters, numbers,
  `_`, or `-`;
- `fixture: local_review_channel`: the isolated seeded state;
- recording policy;
- ordered steps;
- explicit termination and state removal.

Every step must contain an action and an observed postcondition. A step may also
provide ordered fallback locators, a timeout, a sustained assertion, or one
named measurement:

```yaml
- name: open_search
  act: {type: press, key: k, modifiers: [command]}
  expect: {exists: {id: search-dialog}}
  timeout_ms: 5000
  measure: search_open_latency
```

### Locators

Prefer a stable semantic `id`. Add a role/name fallback when the native
Accessibility tree exposes one:

```yaml
locate:
  - {id: message-input}
  - {role: text-area, name: Message}
```

Do not use screen coordinates, styling classes, translated prose when a stable
identifier is available, or a broad role such as `button` without a name. If a
production control lacks a stable identity, add a narrowly named semantic/test
identifier to that control. Do not add review-only behavior merely to make a
selector pass.

### Actions and assertions

Supported actions are `activate`, `click`, `move_pointer`, `press`, `scroll`,
`type_text`, and `wait`. Keyboard modifiers are `command`, `control`, `option`,
and `shift`.

Supported assertions cover element existence/nonexistence, focus, enabled state,
text value, and scroll bounds. `expect_for` requires an assertion to remain true
for a duration; use it for dwell or stability behavior rather than a blind
sleep. Keep waits short and use a postcondition that proves the user-visible
state actually changed.

### Required mutation proof

A new regression journey is not proven by a green run alone. Deliberately
reintroduce the guarded defect or mutate its critical locator/postcondition and
show that the journey fails for the intended reason. Then restore the source and
show it passes. Existing examples live in `tests/fixtures/broken-*.yaml`.

Record both outcomes:

- green receipt with visible artifact and passing cleanup;
- failed receipt with causal diagnostic, preserved screenshot/video/semantic
  evidence, and passing cleanup.

## Author an iOS journey

iOS journeys are Flutter integration tests under `mobile/integration_test/`.
Use `native_review_pairing_test.dart` as the template and
`test_driver/integration_test.dart` as the shared host driver.

Guidelines:

1. Exercise a real app surface with `IntegrationTestWidgetsFlutterBinding`.
2. Select controls by stable `Key`, not coordinates or incidental text.
3. Assert state before and after each actuation.
4. Use `pumpAndSettle()` for animations and bounded `pump(Duration(...))` only
   when the state needs to remain visible in the recording.
5. Keep secrets and production relay state out of fixtures.
6. Pass a different test without changing the runner:

   ```bash
   ./tools/native-review/bin/review-ios \
     --device 'iPhone 17 Pro' \
     --test mobile/integration_test/your_journey_test.dart
   ```

The runner erases, boots, and waits for the selected simulator; records before
launch; invokes `flutter drive --keep-app-running`; captures the final app
screen; finalizes the recorder; then shuts down and erases the device even after
failure. Add runner behavior through unit-tested helpers rather than shelling
around this lifecycle.

Mutation-test an iOS journey by temporarily removing/changing the critical
widget key or expected state. Confirm nonzero exit, exact Flutter diagnostic,
finalized MP4, screenshot/log receipt entries, and passing cleanup. Restore the
source before committing.

## Performance comparison and budgets

A Desktop step with `measure: <name>` persists its complete native
action-to-observed-postcondition duration. During the journey the harness also
samples app CPU and resident memory every 100 ms. Capture a cohort rather than
trusting one noisy laptop run:

```bash
just native-review-benchmark tools/native-review/desktop/tooltip-fresh-dwell.yaml 5
```

Compare at least three clean baseline receipts with at least three clean
candidate receipts using a checked-in budget policy:

```bash
./tools/native-review/bin/review-native compare \
  --baseline /path/base-1/receipt.json \
  --baseline /path/base-2/receipt.json \
  --baseline /path/base-3/receipt.json \
  --candidate /path/head-1/receipt.json \
  --candidate /path/head-2/receipt.json \
  --candidate /path/head-3/receipt.json \
  --budget tools/native-review/performance/tooltip-fresh-dwell.yaml \
  --output test-results/native-review/performance-comparison.json
```

A budget's `flow` must match the journey. Each metric may set an absolute `max`,
a `max_regression_percent`, or both. Establish ceilings from repeated known-good
runs on representative hardware; do not choose a ceiling merely because the
current candidate slips under it. Name interaction measurements for the user
transition they represent, such as `search_open_latency`.

Comparison uses cohort medians and preserves raw samples/min/max. It fails closed
for dirty runs, failed cleanup, mixed source revisions, wrong flows, missing
metrics, too few samples, incompatible machine/OS fingerprints, or breached
budgets. Run baseline and candidate on the same host under comparable load.
Recording overhead is intentionally present in both cohorts.

CPU/RSS sampling measures the app process during the foreground journey. It does
**not** by itself prove background-idle CPU, relay RPS, reconnect storms,
serialization volume, relay saturation, or cross-client scaling. Those require
seeded workload journeys plus the relevant counters/traces before a budget can
guard them.

## Before opening a PR

- Run the full package/repository gates required by `AGENTS.md` and `TESTING.md`.
- Run the native-review Python suite.
- Mutation-prove each new regression guard.
- Commit, confirm a clean tree, and run the native journey at that exact HEAD at
  least twice for lifecycle-sensitive work.
- Inspect the MP4 and screenshot; a passing log attached to SpringBoard or the
  wrong app is not visual evidence.
- Verify receipt SHA, `dirty: false`, selected device/runtime, artifact files,
  and cleanup status.
- After merging `origin/main`, rerun any proof whose HEAD changed.

## Troubleshooting

- **Doctor denies Accessibility or Screen Recording:** grant the permission to
  the actual invoking terminal/agent binary, restart it, and rerun doctor.
- **Desktop relay refused:** use the isolated loopback relay on port 3030. Remote
  relays intentionally fail closed.
- **No iOS device with that name:** inspect `xcrun simctl list devices available`
  and pass an exact available name. When several runtimes contain that model,
  the runner selects the newest runtime and records it.
- **Notification prompt covers iOS:** run through `review-ios`; it provides both
  the Dart define and simulator child environment. Direct `flutter drive` does
  not provide the complete review contract.
- **Receipt passed but evidence looks wrong:** treat it as a harness defect. The
  recording and screenshot are part of the assertion surface, not decoration.
- **Push hooks cannot find Flutter/Rust:** activate Hermit and ensure repository
  `bin` remains on `PATH`, for example `PATH="$PWD/bin:$PATH" git push ...`.
