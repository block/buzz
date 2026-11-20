# buzz-feature-flags

Provider-neutral, typed feature-flag contracts for Buzz server crates.

This crate is infrastructure-only. It defines a small API for flag evaluation,
plus a static evaluator and an optional LaunchDarkly adapter behind a compile
feature.

## Purpose

`buzz-feature-flags` centralizes typed flag evaluation so server crates can use
flags without importing vendor SDK types.

Provider-neutral types:

- `BooleanFlag`
- `IntegerFlag` (`i64`, including negative values)
- `EvaluationContext` (a `CommunityId` with an optional actor `PublicKey`, or
  `for_process()` for process-wide settings such as concurrency caps)
- `FlagEvaluator` (typed `evaluate_bool` and `evaluate_int`)
- `StaticEvaluator` (always returns declared defaults)
- `EnvironmentEvaluator` (process-wide immutable environment snapshot)
- `EnvironmentDiagnostic` (sanitized malformed-value report)

LaunchDarkly support is optional and compile-gated behind
`buzz-feature-flags/launchdarkly`.

`EnvironmentEvaluator` is provider-neutral and always available (no Cargo
feature and no external dependency).

## Environment Variable Naming

`EnvironmentEvaluator` reads only the fixed namespace
`BUZZ_FEATURE_FLAG_`.

For each flag key:

- uppercase ASCII letters,
- replace each run of non-ASCII-alphanumeric characters with `_`,
- trim leading/trailing `_`,
- prepend `BUZZ_FEATURE_FLAG_`.

Example: `relay.feature.query-v2` → `BUZZ_FEATURE_FLAG_RELAY_FEATURE_QUERY_V2`.

Normalized names must be unique across declared flags. Collisions (for example
`a-b` and `a_b`) intentionally map to the same environment variable. Enforcing
uniqueness belongs in a future flag registry; this crate does not add a
registry yet.

## Relay Provider Selection

`buzz-relay` builds exactly one evaluator at startup, chosen by
`BUZZ_FEATURE_FLAGS_PROVIDER` (outside the `BUZZ_FEATURE_FLAG_` namespace, so
it can never be read as a flag):

| Value | Evaluator |
|-------|-----------|
| unset, `static` | `StaticEvaluator` |
| `environment` | `EnvironmentEvaluator::from_process_environment()` |
| `launchdarkly` | `LaunchDarklyEvaluator`, from `BUZZ_LAUNCHDARKLY_SDK_KEY` and the optional Relay Proxy URL `BUZZ_LAUNCHDARKLY_RELAY_PROXY_URL` |

The value is parsed into one provider; there is no stacking, no precedence and
no fallback from one provider to another, and the evaluator never changes for
the life of the process. Every relay build carries the LaunchDarkly adapter
(`buzz-feature-flags/launchdarkly`), which stays inert unless selected, so one
published artifact serves every deployment and the deployment's configuration
pins its provider.

The composition root lives in `crates/buzz-relay/src/feature_flags.rs`. It
constructs the evaluator once and hands consumers the shared
`Arc<dyn FlagEvaluator>`, keeping the concrete owner to drain environment
diagnostics and to call LaunchDarkly's `close()` at shutdown.

Startup fails on a local configuration error: an unknown provider,
`launchdarkly` without an SDK key, or a malformed Relay Proxy URL. It does not
fail when LaunchDarkly is unreachable: the relay waits briefly for flag data,
then serves declared defaults while the SDK keeps connecting, and
`buzz_feature_flags_ready{provider}` reports whether the provider has
initialized. Once initialized, LaunchDarkly serves its last-known values
through a later outage, so the gauge stays 1 and flag values do not revert to
defaults until the SDK reconnects and receives new data.

## `buzz-db` Boundary (approved correction)

`buzz-db` may evaluate flags internally when choosing between equivalent query
implementations. It should depend on `buzz-feature-flags` without enabling the
LaunchDarkly feature, and it should never construct or import provider types.

Concise example (query path selection only):

```rust
use buzz_core::CommunityId;
use std::sync::Arc;

use buzz_feature_flags::{EvaluationContext, FlagEvaluator, IntegerFlag};

pub struct Db {
    feature_flags: Arc<dyn FlagEvaluator>,
}

impl Db {
    pub async fn list_events(
        &self,
        community: CommunityId,
    ) -> anyhow::Result<Vec<EventRow>> {
        let context = EvaluationContext::for_community(community);

        let query_version = self.feature_flags.evaluate_int(
            IntegerFlag::new("db.events.query-version", 1),
            &context,
        );

        if query_version >= 2 {
            self.list_events_v2_sql(community).await
        } else {
            self.list_events_v1_sql(community).await
        }
    }
}
```

SQL details are intentionally omitted in this example; the key point is that
public `Db` methods accept only domain inputs, while feature-flag evaluator
wiring remains internal to `Db` construction.

`EvaluationContext::for_actor` is appropriate only when the existing DB
operation already receives an authoritative authenticated actor for domain
behavior. Feature targeting must not add actor/pubkey parameters to otherwise
actor-free DB APIs.

For LaunchDarkly, a community evaluation always includes kind `community` plus
optional global kind `pubkey`; a process evaluation is a single kind `relay`
context keyed `relay`. Percentage rollouts must explicitly choose a
kind present in evaluation: rollouts must set `contextKind` to a kind present
in evaluation (`community` or `pubkey`) when they are intended to bucket Buzz
traffic. LaunchDarkly rollouts where configuration omits `contextKind` defaults
to `user`; Buzz does not synthesize a `user` alias, so those rollouts bucket as
zero and pick the first positive-weight variation.

## Guardrails

- Evaluation context inputs are authoritative server-resolved values
  (`community`, optional `actor`), not client-supplied targeting attributes.
- Flags may choose between **equivalent** implementations only, with one
  exception: an operational limit (a concurrency cap) may be set by a flag
  when the flag can only *tighten* it. The consumer clamps the value to a
  ceiling fixed at startup, and a missing, non-positive or unavailable value
  means the ceiling. Shedding under such a limit must be retryable.
- Flags must not weaken authorization, tenant isolation, ordering guarantees,
  transaction/consistency behavior, or schema invariants.
- Declared defaults choose the established safe path.
- Integer values must be validated at the consumer boundary before affecting
  behavior.

## Fallback, Startup, and Lifecycle

- `StaticEvaluator` always returns each flag's declared default.
- `EnvironmentEvaluator` returns declared defaults when values are missing,
  empty, invalid, non-Unicode, or the normalized flag key is unusable. Present
  malformed values also produce a sanitized diagnostic; missing values do not.
- LaunchDarkly adapter returns declared defaults when a flag is missing, wrong
  type, or evaluation fails.
- LaunchDarkly numbers are SDK `f64` values. Integer evaluation accepts only
  mathematically integral values that convert to `i64` exactly from that `f64`
  representation; otherwise it returns each flag's declared default.
- Startup chooses one evaluator from configuration (`StaticEvaluator`,
  `EnvironmentEvaluator`, or LaunchDarkly); provider precedence/stacking is out
  of scope.
- If LaunchDarkly is used, call evaluator `close()` during process shutdown.
  `close()` blocks the calling thread while it flushes analytics and has no
  deadline of its own, so async shutdown code should run it off the runtime
  and bound the wait (the relay gives it a detached thread and 2 s).
- Safety invariants must never rely on remote-flag availability.

## Build and Test Expectations

Build modes:

```bash
# Provider-neutral (default): no LaunchDarkly dependency activated
cargo build -p buzz-feature-flags

# Provider-neutral environment evaluator is included in the default build
cargo test -p buzz-feature-flags --test environment_evaluator

# LaunchDarkly adapter enabled
cargo build -p buzz-feature-flags --features launchdarkly
```

Dependency-graph expectation checks:

```bash
# Default graph should exclude launchdarkly-server-sdk
cargo tree -p buzz-feature-flags

# Feature graph should include launchdarkly-server-sdk
cargo tree -p buzz-feature-flags --features launchdarkly
```

Testing expectations:

- Run provider-neutral tests and LaunchDarkly-feature tests for this crate.
- When `buzz-db` adopts flag-gated query selection, add parity tests proving
  old/new query paths return equivalent rows, ordering, and transactional
  behavior for the same inputs.

## References

- [ARCHITECTURE.md](../../ARCHITECTURE.md)
- [docs/multi-tenant-relay.md](../../docs/multi-tenant-relay.md)
