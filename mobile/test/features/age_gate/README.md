# Age-check contract

Normal app, relay, and push startup do not depend on an age result. The only
operation that can restrict access is a valid under-18 response from the current
launch request. Errors, malformed responses, timeout, disposal, and late callbacks
leave access allowed. There is one attempt per provider lifecycle, with no
blocking retry or restart flow.

Production enforcement remains disabled. Dogfood builds can opt in with the
Flutter define `BUZZ_AGE_GATING_ENABLED=true`. This enables the same notifier used
by the native-channel and app tests. No real-device build is a PR prerequisite;
actual OS prompts, signing, and account behavior will be checked in dogfood.

## Automated evidence

- `age_signal_provider_test.dart`: real method-channel codec, allowed/under-18
  boundaries, malformed payloads, exceptions, missing plugin, timeout, late
  completion, concurrent callers, invalidation, disposal, and the build opt-in.
- `age_gate_app_test.dart`: full app and push bootstrap remain mounted while the
  native response is pending or fails, and unmount only for a current under-18
  response. Notification restoration failure does not gate app access.
- Notification restriction leaves saved snapshots, signing keys, preferences,
  and remote leases intact. Only the confirming app process holds restriction
  authority; a later launch does not need storage restoration to allow access.
- Swift and Kotlin adapter tests: platform-specific bounds are normalized
  conservatively, including contradictory ranges and extreme integers. Android
  SDK task tests exercise shared access, minor response delivery, both failure
  stages, and retired callbacks through the production request coordinator.
- iOS simulator RunnerTests: the production method handlers deliver minor/error
  callbacks, allow notification restoration without app-group storage, and
  preserve snapshots through restriction, cleanup failure, and restoration.
- Swift notification tests: real cross-process lock contention suppresses
  presentation. Process exit releases restriction without cleanup. Missing, stale,
  and inaccessible lock files cannot establish a restriction.

Run the Flutter age-gate suite both with and without the dogfood define. A test
must fail if pending/error becomes restricted or an expired request can restrict.
The implementation was mutation-tested for pending and error blocking.

An already-running platform consent dialog cannot be proven dismissible by Dart
unit tests. That behavior remains part of dogfood validation before general
re-enablement. Old releases that permanently saved push opt-out preferences did
not record their cause; this change does not guess which opt-outs to reverse.
