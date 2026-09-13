# buzz-acp: add a TeamContextProvider registry seam for team_instructions

Implements #3351. Third and smallest of a trio of registry-seam proposals.
#3167 (relay kind registry) and #3280 (desktop channel-feature registry)
are being submitted separately.

## Problem

Every source that wants to contribute to an agent's standing
`team_instructions` context (workspace house rules, per-team norms,
channel-scoped policy, agent memory, future capability-carried context)
has to be hand-wired into the same spot in `buzz-acp`'s startup path. That's
a central chokepoint that gets harder to extend as the agent surface grows,
the same pattern #3167 and #3280 address on the relay and desktop client.

**Note on scope versus the RFC text.** #3351 was written against a fork
that had already added a workspace "Agent Guidelines" fetch at this call
site. That fetch doesn't exist in `block/buzz` upstream. At the current
call site in `tokio_main`, `team_instructions` is just
`config.team_instructions.clone()`, a plain pass-through with nothing to
generalize. So this PR ships the seam itself with an empty built-in
provider list, rather than porting a provider that has no upstream
counterpart. The registry, trait, and fold function are what the RFC
proposes. Only "the existing Agent Guidelines fetch becomes one registered
provider" is deferred, since there's nothing upstream yet to convert.
Wiring a first real provider (workspace house rules, for example) is
natural follow-up work once this seam lands.

## How

- New `crates/buzz-acp/src/team_context.rs`:
  - `TeamContextProvider` trait: `name()` plus
    `provide(ctx) -> Option<String>`, object-safe via
    `Pin<Box<dyn Future<...> + Send>>`, the same pattern
    `buzz_workflow::ActionSink` already uses, so no `async-trait` dependency.
  - `TeamContextCtx`: carries `relay_url` and `keys`, the inputs a
    relay-backed provider needs.
  - `builtin_team_context_providers()`: the ordered list run at startup.
    Empty today, see the note above.
  - `build_team_instructions(providers, ctx, base)`: runs providers in
    order, drops `None`/blank contributions, joins the rest ahead of `base`
    with a blank line between sections. With an empty provider list this is
    the identity on `base`.
- `crates/buzz-acp/src/lib.rs`: the `tokio_main` startup call site now
  builds `team_instructions` via
  `team_context::build_team_instructions(...)` instead of
  `config.team_instructions.clone()`.

**Behavior-preserving.** `config.team_instructions` is already trimmed and
empty-filtered at config-parse time, so `build_team_instructions`'s own
trim/empty-filter on `base` is a no-op there. With the empty provider list,
the new call site produces the same output as the old
`config.team_instructions.clone()` for every input it can receive today.

## Review focus

1. The RFC issue (#3351) describes generalizing an existing Agent
   Guidelines fetch that doesn't exist in this repo. Should the issue text
   itself be corrected to match, so future readers aren't confused by a
   motivation section describing code that isn't there?
2. Is landing the seam with an empty provider list (proven by
   `empty_providers_is_identity_on_base`) an acceptable way to merge this
   ahead of any real provider, or would you rather see it land together
   with a first provider so the non-empty path is exercised in production
   immediately?

## Testing

At commit `af6c2f0dc` (the last code commit on this branch; later commits
only touch this description):

- `cargo test -p buzz-acp`: 805 passed, 0 failed, existing suite untouched.
- New unit tests in `team_context.rs`:
  - `empty_providers_is_identity_on_base`: with today's empty registry, the
    fold reproduces `base` exactly (or `None` for blank/absent input). This
    is the property that makes the change behavior-preserving.
  - `builtin_providers_start_empty`.
  - `providers_fold_in_order_ahead_of_base`: ordering, blank-line joins,
    `None`/whitespace-only contributions dropped, using a network-free stub
    provider.
  - `all_empty_yields_none`.
- `cargo clippy -p buzz-acp --all-targets -- -D warnings`: clean.
- `cargo fmt -p buzz-acp -- --check`: clean.
- No new `unwrap()` or `unsafe` in production code.

## Follow-ups (not in this PR)

- Register a first real provider once one exists upstream, such as a
  workspace house-rules or guidelines fetch, to exercise the non-empty
  registry path in production.
- #3167 and #3280 (relay kind registry, desktop channel-feature registry),
  submitted separately.

## Duplicate check

```
gh search issues --repo block/buzz "agent context provider OR team_instructions"
gh search prs --repo block/buzz "agent context provider OR team_instructions"
gh search issues --repo block/buzz "TeamContextProvider"
```

Only hit: this PR's own RFC, #3351. No duplicate issues or PRs found.
