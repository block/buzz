# Relay Concurrency Caps

Two per-pod caps can be tightened while the relay runs, through integer
feature flags evaluated in the process context (see
`crates/buzz-relay/src/concurrency_limits.rs`):

| Flag | Caps | Ceiling |
|------|------|---------|
| `buzz.relay.concurrency.history-reads` | subscription-history reads holding a reader-pool connection | the reader-pool share reserved for history |
| `buzz.relay.concurrency.auth` | verified NIP-42 AUTH frames in their database checks | none |

A positive value sets the cap, clamped to the ceiling. Zero, a negative value
or no value leaves the cap at its ceiling. A flag can only tighten a cap.
Each pod re-reads the flags every 5 s.

Past the history-read cap, a read waits up to 1 s and is then refused as
retryable. Past the AUTH cap, a login waits up to 2 s and then gets a bare
close, and the client reconnects on its own backoff.

## Tightening a cap

1. Set the flag to a positive value for the deployment's environment.
2. Within about 5 s, check `buzz_concurrency_limit{gate}` on every pod
   (`history_reads` or `auth`). It shows the applied limit, with `0` meaning
   no limit.
3. Watch the sheds: `buzz_auth_outcomes_total{outcome="concurrency_limit"}`
   for AUTH.

To release, set the flag back to `0`.

## When a tightened cap loosens

A cap returns to its ceiling, without an error, when:

- **The flag value cannot be read as an integer.** A fractional, string or
  otherwise mistyped value evaluates to the default `0`. Change values only
  among the flag's defined numeric variations.
- **The flag is deleted or turned off with an off variation of `0`.**
- **A pod starts while the flag provider is unreachable.** The relay waits up
  to 3 s for flag data, then serves defaults until the provider connects. A
  pod restarted during a provider outage therefore comes up at the ceiling,
  even while the cap is tightened for the pods that were already running.
  `buzz_feature_flags_ready{provider} == 0` marks such a pod.

A pod that has already received flag data keeps its last-known values through
a later provider outage, so a running pod's cap does not loosen when the
connection drops.

During an incident in which the provider may be down, confirm
`buzz_concurrency_limit` on each pod after every restart or scale-up, not just
after the flag change.

## Health signals

- `buzz_feature_flags_ready{provider}`: `1` once the provider has delivered
  data (always `1` for the `static` and `environment` providers).
- `buzz_concurrency_limit_refresh_panics_total`: a refresh that panicked. The
  next refresh retries; a rising count means the caps are not following the
  flags.
- Shutdown waits at most 2 s for the provider's final analytics flush.
