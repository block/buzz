// Canonical `ReconnectHookConfig` definition, `include!`d into BOTH
// `build.rs` (compile-time validation) and `commands/relay_reconnect.rs`
// (runtime deserialization). build scripts cannot import from the crate, so
// sharing the source via `include!` is what guarantees the build-time check
// and the runtime parse use an identical schema — zero drift surface.
//
// Keep this file dependency-free: only `serde` derives, no crate-internal
// imports. Both consumers have `serde` available.

/// Typed config carried by the build-time env var `BUZZ_BUILD_RELAY_RECONNECT_CMD`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // build.rs only constructs it for validation
pub struct ReconnectHookConfig {
    /// Ordered commands to run. Each inner vec is [program, arg1, arg2, ...].
    pub steps: Vec<Vec<String>>,
    /// Command whose stdout is polled for readiness after steps complete.
    pub ready_probe: Vec<String>,
    /// RAW SUBSTRING matched in the probe's stdout to consider the transport
    /// ready — NOT a parsed field. Pick a token unlikely to collide with other
    /// substrings in the probe output (e.g. `warp-cli -j status` JSON, where
    /// "Connected" can collide with "Connecting"/"Disconnected").
    pub ready_match: String,
    /// Per-process wall-clock cap (ms) for each step and the readiness probe
    /// phase; non-fatal on expiry — the generic relay reconnect still fires.
    pub timeout_ms: u64,
}
