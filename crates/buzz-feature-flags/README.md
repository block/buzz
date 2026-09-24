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
`a-b` and `a_b`) intentionally map to the same environment variable. Enforcing
uniqueness belongs in a future flag registry; this crate does not add a
registry yet.

## Relay Artifact Contract (not yet wired into `buzz-relay`)

The intended composition root is the outer relay artifact. It must select
exactly one provider at compile time:

- `static-feature-flags`
- `environment-feature-flags`
- `launchdarkly-feature-flags`

`launchdarkly-feature-flags` is the only relay feature that forwards to
`buzz-feature-flags/launchdarkly`. `static-feature-flags` and
`environment-feature-flags` keep the reusable crate vendor-neutral and still
allow adapter code to compile in tests.

The relay artifact boundary, not this reusable crate, owns exact-one
enforcement. Consumer integration PRs must carry `exact-one compile_error guards`
verbatim/equivalently. That keeps `buzz-feature-flags` free to compile multiple
adapters for tests while ensuring a staging or production relay artifact cannot
choose a different evaluator at runtime.

This repository currently adds the crate and adapter, but does **not** yet wire
relay `AppState` or handlers to consume it. `crates/buzz-relay/Cargo.toml`
still has no dependency on `buzz-feature-flags`, so current workspace/default
builds remain intentionally unchanged. The executable contract for the future
relay integration lives in
`tests/fixtures/relay-feature-selection`.

## Composition Root Shape (artifact boundary)

Construct once at process startup, then inject shared `Arc<dyn FlagEvaluator>`
into server components.

```rust
use std::sync::Arc;
use std::time::Duration;

use buzz_feature_flags::{EnvironmentEvaluator, FlagEvaluator, StaticEvaluator};
#[cfg(feature = "launchdarkly-feature-flags")]
use buzz_feature_flags::launchdarkly::{
    LaunchDarklyEvaluator, LaunchDarklyInitError, LaunchDarklyRuntimeConfig,
    LaunchDarklyStartError,
};

#[cfg(not(any(
    feature = "static-feature-flags",
    feature = "environment-feature-flags",
    feature = "launchdarkly-feature-flags",
)))]
compile_error!(
    "select exactly one relay feature flag provider feature: static-feature-flags, environment-feature-flags, or launchdarkly-feature-flags"
);

#[cfg(any(
    all(feature = "static-feature-flags", feature = "environment-feature-flags"),
    all(feature = "static-feature-flags", feature = "launchdarkly-feature-flags"),
    all(feature = "environment-feature-flags", feature = "launchdarkly-feature-flags"),
))]
compile_error!(
    "select exactly one relay feature flag provider feature: static-feature-flags, environment-feature-flags, or launchdarkly-feature-flags"
);

#[cfg(any(feature = "static-feature-flags", feature = "environment-feature-flags"))]
#[derive(Default)]
struct FeatureFlagRuntimeConfig;

#[cfg(feature = "launchdarkly-feature-flags")]
struct FeatureFlagRuntimeConfig {
    sdk_key: String,
    relay_proxy_endpoint: Option<String>,
}

struct RelayCompositionRoot {
    feature_flags: Arc<dyn FlagEvaluator>,
    environment_diagnostics: Option<Arc<EnvironmentEvaluator>>,
    #[cfg(feature = "launchdarkly-feature-flags")]
    launchdarkly_lifecycle: Option<Arc<LaunchDarklyEvaluator>>,
}

#[cfg(feature = "launchdarkly-feature-flags")]
enum RelayStartupError {
    LaunchDarklyInit(LaunchDarklyInitError),
    LaunchDarklyStart(LaunchDarklyStartError),
}

#[cfg(feature = "static-feature-flags")]
fn build_feature_flag_evaluator(_config: FeatureFlagRuntimeConfig) -> RelayCompositionRoot {
    RelayCompositionRoot {
        feature_flags: Arc::new(StaticEvaluator),
        environment_diagnostics: None,
    }
}

#[cfg(feature = "environment-feature-flags")]
fn build_feature_flag_evaluator(config: FeatureFlagRuntimeConfig) -> RelayCompositionRoot {
    let _ = config;
    let owner = Arc::new(EnvironmentEvaluator::from_process_environment());
    let feature_flags: Arc<dyn FlagEvaluator> = owner.clone();
    RelayCompositionRoot {
        feature_flags,
        environment_diagnostics: Some(owner),
    }
}

#[cfg(feature = "launchdarkly-feature-flags")]
async fn build_feature_flag_evaluator(
    config: FeatureFlagRuntimeConfig,
) -> Result<RelayCompositionRoot, RelayStartupError> {
    let mut runtime_config = LaunchDarklyRuntimeConfig::new(config.sdk_key);
    if let Some(relay_proxy_endpoint) = config.relay_proxy_endpoint {
        runtime_config = runtime_config.with_relay_proxy_endpoint(relay_proxy_endpoint);
    }

    let owner = Arc::new(
        LaunchDarklyEvaluator::from_runtime_config(runtime_config)
            .map_err(RelayStartupError::LaunchDarklyInit)?,
    );
    owner
        .start_with_default_executor_and_wait(Duration::from_secs(5))
        .await
        .map_err(RelayStartupError::LaunchDarklyStart)?;

    let feature_flags: Arc<dyn FlagEvaluator> = owner.clone();
    Ok(RelayCompositionRoot {
        feature_flags,
        environment_diagnostics: None,
        launchdarkly_lifecycle: Some(owner),
    })
}
```

Each relay feature produces exactly one `build_feature_flag_evaluator` path, and
runtime config carries only the selected provider's settings. There is no
runtime provider enum, no provider-name environment variable, and no fallback
selector.

Startup is fail-closed: if the selected provider fails initialization (including
`start_with_default_executor_and_wait` for LaunchDarkly), relay startup must
return an error and stop rather than falling back to another evaluator.

The extra concrete `Arc` is an owner/lifecycle handle to that same evaluator,
not a layered provider. Consumers receive only the cloned `Arc<dyn
FlagEvaluator>` view. Retain the environment owner to drain diagnostics after
evaluations, and retain the LaunchDarkly owner so shutdown can call `close()`.

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

For LaunchDarkly, the adapter context always includes kind `community` plus
optional global kind `pubkey`. Percentage rollouts must explicitly choose a
kind present in evaluation: rollouts must set `contextKind` to a kind present
in evaluation (`community` or `pubkey`) when they are intended to bucket Buzz
traffic. LaunchDarkly rollouts where configuration omits `contextKind` defaults
to `user`; Buzz does not synthesize a `user` alias, so those rollouts bucket as
zero and pick the first positive-weight variation.

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
- LaunchDarkly numbers are SDK `f64` values. Integer evaluation accepts only
  mathematically integral values that convert to `i64` exactly from that `f64`
  representation; otherwise it returns each flag's declared default.
- Startup chooses one evaluator at compile time (`StaticEvaluator`,
  `EnvironmentEvaluator`, or LaunchDarkly); provider precedence/stacking is out
  of scope.
- If LaunchDarkly is used, call evaluator `close()` during process shutdown.
  `close()` blocks the calling thread while it flushes analytics, so async
  shutdown code should offload it to a blocking shutdown path.
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
- Run the relay-artifact compile contract in
  `tests/relay_feature_selection_contract.rs`, including zero-feature and
  multiple-feature rejection.
- Keep `tests/fixtures/relay-feature-selection/Cargo.lock` checked in for the
  nested `--locked` compile contract. Regenerate it from repo root with:

```bash
cargo generate-lockfile --manifest-path crates/buzz-feature-flags/tests/fixtures/relay-feature-selection/Cargo.toml
```

- When `buzz-db` adopts flag-gated query selection, add parity tests proving
  old/new query paths return equivalent rows, ordering, and transactional
  behavior for the same inputs.

## References

- [ARCHITECTURE.md](../../ARCHITECTURE.md)
- [docs/multi-tenant-relay.md](../../docs/multi-tenant-relay.md)
