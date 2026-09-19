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
- `EvaluationContext` (required `CommunityId`, optional actor `PublicKey`)
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
`a-b` and `a_b`) intentionally map to the same environment variable.

## Proposed Integration Boundary (not yet wired)

The intended composition root is the outer relay binary. It chooses exactly one
evaluator at startup:

- Public/OSS relay build: compile and construct `StaticEvaluator` only.
- Process-environment build mode: construct `EnvironmentEvaluator` from an
  immutable startup snapshot.
- Block-internal relay build (`bb-block`): forward a relay Cargo feature to
  `buzz-feature-flags/launchdarkly`, then construct
  `LaunchDarklyEvaluator` from explicit runtime config (SDK key and optional
  relay proxy endpoint).

Do not layer providers (for example environment-over-LaunchDarkly). Startup
selects one evaluator.

This repository currently adds the crate and adapter, but does **not** yet wire
relay `AppState`/handlers to consume it.

## Composition Root Shape (proposed)

Construct once at process startup, then inject shared
`Arc<dyn FlagEvaluator>` into server components.

```rust
use std::sync::Arc;

use buzz_feature_flags::{EnvironmentEvaluator, FlagEvaluator, StaticEvaluator};
#[cfg(feature = "launchdarkly")]
use buzz_feature_flags::launchdarkly::{LaunchDarklyEvaluator, LaunchDarklyRuntimeConfig};

enum ProviderMode {
    Static,
    Environment,
    #[cfg(feature = "launchdarkly")]
    LaunchDarkly(LaunchDarklyRuntimeConfig),
}

struct RelayCompositionRoot {
    feature_flags: Arc<dyn FlagEvaluator>,
    environment_diagnostics: Option<Arc<EnvironmentEvaluator>>,
    #[cfg(feature = "launchdarkly")]
    launchdarkly_lifecycle: Option<Arc<LaunchDarklyEvaluator>>,
}

impl RelayCompositionRoot {
    fn new(mode: ProviderMode) -> Self {
        match mode {
            ProviderMode::Static => Self {
                feature_flags: Arc::new(StaticEvaluator),
                environment_diagnostics: None,
                #[cfg(feature = "launchdarkly")]
                launchdarkly_lifecycle: None,
            },
            ProviderMode::Environment => {
                let owner = Arc::new(EnvironmentEvaluator::from_process_environment());
                let feature_flags: Arc<dyn FlagEvaluator> = owner.clone();
                Self {
                    feature_flags,
                    environment_diagnostics: Some(owner),
                    #[cfg(feature = "launchdarkly")]
                    launchdarkly_lifecycle: None,
                }
            }
            #[cfg(feature = "launchdarkly")]
            ProviderMode::LaunchDarkly(config) => {
                let owner = Arc::new(
                    LaunchDarklyEvaluator::from_runtime_config(config)
                        .expect("validate explicit runtime config"),
                );
                let feature_flags: Arc<dyn FlagEvaluator> = owner.clone();
                Self {
                    feature_flags,
                    environment_diagnostics: None,
                    launchdarkly_lifecycle: Some(owner),
                }
            }
        }
    }

    fn emit_environment_diagnostics(&self) {
        if let Some(owner) = &self.environment_diagnostics {
            for diagnostic in owner.take_diagnostics() {
                tracing::warn!(?diagnostic, "invalid environment feature flag value");
            }
        }
    }

    #[cfg(feature = "launchdarkly")]
    fn close(&self) {
        if let Some(owner) = &self.launchdarkly_lifecycle {
            owner.close();
        }
    }
}
```

Each branch constructs exactly one evaluator. The extra concrete `Arc` is an
owner/lifecycle handle to that same evaluator, not a layered provider. Consumers
receive only the cloned `Arc<dyn FlagEvaluator>` view. Retain the environment
owner to drain diagnostics after evaluations, and retain the LaunchDarkly owner
so shutdown can call `close()`.

`EnvironmentEvaluator` snapshots environment values once at construction,
retains only `BUZZ_FEATURE_FLAG_` entries, ignores `EvaluationContext`, and
never rereads process environment during evaluation.

For deterministic tests or embedding, `EnvironmentEvaluator::from_pairs(...)`
constructs the same immutable snapshot from explicit key/value pairs.

Environment entries are untyped at construction, so malformed present values
are detected when a typed flag is first evaluated. `take_diagnostics()` drains
each sanitized diagnostic at most once per flag key and expected type. The
composition root should retain the concrete environment evaluator and emit each
drained diagnostic as a warning through the application's logging stack.
Diagnostics contain no raw value or environment-variable name; the value
snapshot itself remains immutable.

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

## Guardrails

- Evaluation context inputs are authoritative server-resolved values
  (`community`, optional `actor`), not client-supplied targeting attributes.
- Flags may choose between **equivalent** implementations only.
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
- Recommended rollout posture for non-critical flags: if LaunchDarkly startup
  fails, degrade to `StaticEvaluator` rather than failing relay startup.
- Startup chooses one evaluator (`StaticEvaluator`, `EnvironmentEvaluator`, or
  LaunchDarkly); provider precedence/stacking is out of scope.
- If LaunchDarkly is used, call evaluator `close()` during process shutdown.
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
