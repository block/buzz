# Multiverse placement foundation

This is the first slice of the replacement for the exact-run preview in #7145.
It **does not enable remote controls**. `buzz_core::placement` is a zero-I/O
projection for the Desktop lifecycle owner, not a relay sequencer, wire codec,
command executor or durable admission journal.

[Approved design](https://blockcell.sqprod.co/sites/buzz-multiverse-design-8671f76f/)
(SHA-256 `46724375f9913a7da96caabaf2020433e2b06fa8bb2f5f4879e30410f8188f9a`),
sections 5.2 and 7.1, governs this model. Newer signed `created_at` wins; equal
sender timestamps use the **lower** event ID. Same-second races and clock skew
are accepted. This is not last-click/causal order or a finite overlap promise.

## Projection

Given the same relevant valid events for one owner/community/agent:

1. Find the highest-ranked Start across all hosts, S.
2. For target H, compare S with the highest-ranked Stop **for H**.
3. If H's Stop wins, H is desired stopped. Otherwise S selects its host and
   marks every other host stopped. With neither contribution, H is unknown.
4. Desired placement exists only if S's host remains desired running. Never
   fall back to an earlier Start after S's host is stopped.

This is equivalent to folding Start/Stop in signed order: every Start replaces
all earlier selections; only a later Stop of that selected host can clear it.
The implementation uses two scans and constant auxiliary space, not sorted
history execution. Stop X preserves Y's Start identity even if Y is learned
later. A continuation retains that identity only while the same Start remains
selected; unrelated Stop X must not invalidate it.

## Integration boundary

- Authenticate signatures and authorize owner, canonical community, agent and
  executor **before** constructing inputs. Bind order, target and action to the
  same signed event. Do not mix scopes or silently broaden legacy exact-run Stop.
- A new codec must make relevant intent readable by every authorized executor
  that must converge. The old destination-only encrypted command is insufficient
  for X to learn Start Y. Do not give each receiver a differently ordered copy.
- Persist request identity, consumed one-shots and outcomes separately. Projection
  tolerates duplicate intent but does not deduplicate effects or solve bounded
  replay-safe retention. A missing history segment is not proof of no intent.
- `retains_start` is only one part of the effect-boundary guard. Recheck current
  authorization, local process state and durable admission too. Do not launch
  from backfill or replay/resume interrupted operations on Desktop restart.
- Restart does not select a host: resolve the current host, deduplicate, use
  ordinary Desktop Stop, and launch only after success and a current guard.
- Move contributes destination Start only after ordinary Stop success and while
  the Move remains valid. Failed/unconfirmed/interrupted Move stays terminal;
  a late Stop success cannot release Start. Explicit Start remains separate.

Next slices bind a versioned authenticated transport to this projection, retain
owner-private visibility, and add durable admission before ordinary Desktop
controls consume it. Typed keyless launch must integrate the actual broker
contract; proposed PRs #6922/#6967 are not assumed available. No generic signer,
agent-key export, presence-based termination proof or stronger cleanup subsystem.

Validation: `cargo test -p buzz-core placement`, full `buzz-core` tests/doc tests,
and all-target/all-feature Clippy. The projection regressions cover all 720
permutations of each of two six-event histories (distinct times and all tied),
every partial prefix and duplicate full set, plus targeted no-resurrection,
unchanged-target, stale-continuation and fast-clock cases. These are model/API
tests, not native lifecycle or end-to-end acceptance.
