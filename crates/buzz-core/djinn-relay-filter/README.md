# Generating and optimizing the complete live relay filter predicate

This experiment asks the whole structural question: does an event satisfy any subscription filter? Code review found repeated tag scans, temporary `TagKind` strings, and hexadecimal ID conversion in the historical implementation. **No historical correctness bug was found.** No deliberate mutant is presented as a production bug.

The principal contract is [the readable `djinn!` specification](src/spec.rs). It is independent of traversal order and the generated search algorithm. For each filter, IDs, authors, kinds, inclusive timestamp bounds and every tag constraint must all hold. Filters are OR-ed, values for one field are OR-ed, and tag keys are AND-ed. The empty filter list rejects; an empty filter accepts. Absent scalar sets impose no constraint; present empty sets reject. IDs/authors are exact 32-byte identities, kinds are `u16`, and timestamps are `u64`, without signed conversions or sortedness preconditions.

For a `(key, values)` tag selector the contract is:

```text
an event tag has that exact key and its second string is in values
OR
key is lower-case h, no event tag has lower-case h at all,
and the canonical stored channel UUID string is in values.
```

Keys and values are exact case-sensitive UTF-8 bytes. Later tag strings are irrelevant. A missing second string differs from an empty second string. A contentless `h` blocks fallback; uppercase `H` does not. Channel selectors are not parsed as UUIDs. Search remains at Postgres FTS and limit remains at query execution; signatures, authentication, membership and routing are outside this predicate.

## Generation and proof boundary

The installed upstream is pinned to `8a9ae17c9a6cdbc5220f1881f5ad69f7e2d7e047`. `cargo djinn implement --agent codex` generated both [the complete Rust solution](djinn/solution/mod.rs) and [its Lean proofs](djinn/proofs/RelayFilter/Proofs.lean). The outer implementation owner authored the declarative specification, data-only projection, workload, tests and registry plumbing, **not** the Rust matching algorithm or Lean proofs. The initial fully verified implementation has 144 Rust lines and passed all 16 obligations with the standard axiom audit. Actual `cargo djinn optimize --cycles 3 --jobs 1` accepted two candidates and rejected/reverted the third after proof and timing. The [archived original structural objective](evidence/STRUCTURAL-V1.md), which included the later-corrected routing-inconsistent fixtures, improved from 10.297 ms to 7.560 ms per iteration (1.36× versus the first generated implementation), measured by the isolated research compiler. Detailed case measurements, regressions and hashes are recorded in the evidence.

The proof covers the complete generated predicate: OR, all scalar fields, all generic tags, fallback, termination and absence of Rust panics in the translated operation. The supporting specifications are total and the top-level Boolean result is unambiguous; existence is discharged from the proved implementation. Only `propext`, `Classical.choice`, and `Quot.sound` may occur in audited theorem dependencies. There are no added axioms, admissions, weak output contracts or modifications to trusted translation/models.

The data projection, registry/concurrency, allocator, compiler, Nostr types and access-control integration are **not formally proved**. Projection uses the SDK's raw first tag spelling and optional second content, exact identity bytes, unsigned scalar values, and canonical stored UUID text. The SDK semantics were checked against pinned Nostr 0.44.7. The native contract matrix and composed-input differential tests exercise that boundary against the unchanged typed implementation.

This Djinn revision's build gate checks specification tokens and report status, but **does not hash the solution**. A later Cargo build cannot establish proof provenance. After final formatting/source changes, run fresh `cargo djinn verify`, inspect all 16 proved statuses and the axiom audit, and record SHA-256 of the solution, specification, data records, adapter, models, proofs and report.

## Live deployment and cost placement

The isolated demo lives under the core owner (`crates/buzz-core/djinn-relay-filter`) so future regenerated Rust participates in the existing native CI/hook and relay artifact hash boundaries; no CI changes are needed. Its own `[workspace]` and default in-crate `djinn` work directory keep research builds isolated. Normal Buzz builds include the exact [generated source and adapter](../src/prepared_filter.rs); they have no Djinn/Lean/toolchain dependency. The original typed `filter::filters_match` remains the HTTP/history/COUNT/push owner. Only live WebSocket registry fanout uses this complete prepared predicate.

Prepared immutable selectors live beside originals in the authoritative subscription entry. Registration and same-ID replacement prepare once; removal drops both naturally. Original filters still own index maintenance and `get_filters`. Candidate index snapshots still contain IDs only. Fanout checks the current entry's community/scope before projecting the event, lazily once per fanout, and uses the current entry's prepared selectors. Channel revocation changes scope as before. Existing later result-level access controls remain in place. There is no second cache map, index, timer, retry policy or lifecycle owner.

The first generated algorithm compares bytes directly and traverses candidate values/tags without allocating temporary tag-kind or hexadecimal strings. Its scan order differs from the historical algorithm; both early and late positions are measured. The accepted optimizer checks first elements before the reverse remainder and adds single-byte/suffix guards followed by full slice equality. These guards live inside the verified complete operation and impose no ordering, length or normalization preconditions. Worst-case tag/value search remains a product of their sizes, so this is not a claimed universal asymptotic improvement.

See [the full measurement table and tradeoffs](evidence/MEASUREMENTS.md), [tool pins and exact commands](evidence/TOOLCHAIN.md), and [final source/proof/model/report hashes](evidence/final-hashes.json).

## Measurements and validation

Performance results distinguish the complete prepared predicate, predicate plus event projection, the rejected per-call filter/event cloning prototype, and **actual `fan_out_scoped`** with index lookup, current-entry guards/lookups, matching, deduplication and result construction. Registration/replacement and retained prepared memory are reported separately. There is no network, server throughput or whole-app speedup claim.

Corrected live fixtures contain exactly ONE lower-case `h` equal to the stored channel, with `e`/`p`/`t` distractors. This agrees with ingest routing to the first parseable `h`. The primary client shape puts `h` FIRST; later `h` placement remains a valid order stress only with no preceding `h`. Channel selectors use varied canonical UUIDv4 bytes, sorted as the SDK's `BTreeSet` stores them before selecting ranks. Off-scope channels yield zero candidates. Full fanout varies 1/10/100/1000 registered subscriptions, 1/50/128 channels and first/last ranks, plus wrong-kind, other-community and global-`p` controls. A separate eight-case holdout crosses 50/128 selectors, 1/100 subscriptions and either fixed middle rank or a prebuilt fixed-seed sequence of 16 interior ranks, always with the single `h` first. It is excluded from optimization. These are synthetic configurations, not an observed traffic mix, and the events are structural fixtures rather than authenticated traffic.

Two current desktop features create multi-channel live subscriptions. [Huddle presence](../../../desktop/src/features/huddle/HuddlePresenceContext.tsx#L14) collects joined channels; its [runtime batches them](../../../desktop/src/features/huddle/lib/huddlePresenceRuntime.ts#L97) into groups of at most 128 and [subscribes with `#h`](../../../desktop/src/features/huddle/lib/huddlePresenceRuntime.ts#L549). [Hidden-DM resurfacing](../../../desktop/src/features/channels/useDmResurfaceFromMessages.ts#L99), [mounted in AppShell](../../../desktop/src/app/AppShell.tsx#L513), uses the same 128-channel batching. The [relay enforces 128 explicit channel values across the request](../../buzz-relay/src/handlers/req.rs#L1263). A width of 50 models a possible partial batch; it is not evidence of an observed subscription size or traffic distribution.

The original three optimization rounds remain archived in [structural training evidence](evidence/STRUCTURAL-V1.md). Their inconsistent late/miss fixtures are not ordinary live-fanout headlines. The additional real optimization round freezes corrected routing-consistent cases and preserves the unchanged complete specification. No historical production correctness bug was found, and proof-draft/tool failures are not Buzz bugs.

All expected outputs are checked outside timing. Fixture/registry allocation is outside fanout timing; event projection and result allocation remain inside. Release runs use warmup, calibration to at least 40 ms, nine raw samples, `black_box` inputs/outputs and consumed results. Metadata records snapshots, CPU/cgroup quota and compiler. Optimization uses the isolated research compiler; comparative published caller measurements use the same normal pinned compiler for before/after.

Native coverage consists of the exact owner tests, the explicit 39-case full public contract matrix, 10,000 deterministic composed typed inputs, and same-community/same-scope replacement of selectors. Existing registry tests retain scope isolation, revocation, removal, stale candidate snapshots and concurrency coverage. No browser tests were added or removed.

Blox's configured mirror lacks locked `rustls 0.23.46`; normal workspace relay validation on that workstation is blocked without a lock/mirror change. The isolated macOS validation clone at base `71d7aa0` used the normal Hermit Rust 1.95.0 environment and unchanged workspace manifests, lockfiles and registry configuration. On retained solution SHA `c4ddd3602eb5c65e9e04ac639f68cd3d7fb34b468b28f61775638579cab509bd`, normal locked core filter tests passed (11/11), the complete relay subscription test module passed (35/35), and core+relay all-target Clippy with warnings denied, workspace formatting and diff checks passed. All eight runtime files match this export and remained unchanged after those checks; all 25 active source/proof hashes match [the final manifest](evidence/final-hashes.json). [Validation provenance](evidence/parent-validation-provenance.json) separately preserves the earlier in-progress `8faf…` result and compilation-only hook preparation; those are not substituted for the retained-source checks.

The standalone package compiles exact current registry/owner sources with real core types/dependencies. Its UUID 1.28.0/serde_json 1.0.154 differ from workspace UUID 1.23.1/serde_json 1.0.150; pinned Nostr 0.44.7, DashMap 6.2.1, metrics 0.24.6 and chrono 0.4.45 agree. Comparative measurements use the same harness lock. Standalone measurements and focused locked-workspace checks do not establish a live relay workflow, the complete push-hook suites, other platforms or human acceptance. Local relay testing remains deferred because Docker requires organizational sign-in. No services or existing identities were changed.

## Reproduction

Keep the proof package isolated from the normal workspace. Normal source checks use the repository's Hermit environment. Research commands run from this isolated crate after activating the pinned external toolchain. The sibling `rust-djinn` clone dependency is intentionally relative; cache/tool locations are runtime environment, not shipped dependencies. Set `DJINN_CHARON`, `DJINN_AENEAS`, `DJINN_AENEAS_LEAN` to installed pinned tools. [TOOLCHAIN.md](evidence/TOOLCHAIN.md) records the actual Blox locations/commands; adapt those explicit runtime paths on another machine. A fresh nested Codex exec unsets inherited `CODEX_THREAD_ID` and explicitly sets MCP toolchain environment for `RUSTUP_HOME`, `CARGO_HOME`, `ELAN_HOME`, `OPAMROOT`, `DJINN_LAKE_PACKAGES` and `PATH`, without changing credentials or global settings.

```sh
# Normal toolchain, from repository root
source bin/activate-hermit
cargo test --manifest-path crates/buzz-core/djinn-relay-filter/baseline/Cargo.toml --lib
cargo clippy --manifest-path crates/buzz-core/djinn-relay-filter/baseline/Cargo.toml --all-targets -- -D warnings
cargo build --release --manifest-path crates/buzz-core/djinn-relay-filter/baseline/Cargo.toml
# Compare the immutable first generated solution under the same registry/adapter:
cargo build --release --manifest-path crates/buzz-core/djinn-relay-filter/baseline/Cargo.toml --features first-generated --target-dir /tmp/djinn-first-bench
crates/buzz-core/djinn-relay-filter/baseline/target/release/fanout historical
crates/buzz-core/djinn-relay-filter/baseline/target/release/fanout current
crates/buzz-core/djinn-relay-filter/baseline/target/release/relay-filter-baseline
crates/buzz-core/djinn-relay-filter/baseline/target/release/relay-filter-baseline prepared
crates/buzz-core/djinn-relay-filter/baseline/target/release/memory

# Isolated proof tools, from experiment root
source "$DJINN_ENV" # installed pinned research activation; see TOOLCHAIN.md
cargo djinn implement --agent codex --args "$DJINN_RUN_ARGS" --prompt-file <generation-prompt>
cargo djinn optimize --agent codex --cycles 3 --jobs 1 --min-time 5 --batches 9 --threshold 3 --args "$DJINN_RUN_ARGS" --prompt-file <optimization-prompt>
cargo djinn optimize --agent codex --cycles 1 --jobs 1 --min-time 5 --batches 9 --threshold 3 --args "$DJINN_RUN_ARGS" --prompt-file <corrected-optimization-prompt>
cargo djinn verify
```

Raw agent logs and failed-attempt evidence are kept outside tracked files. The first compact nested-closure specification exposed a generated well-formedness scope error; the equivalent declarative-relations form avoids that translation limitation without weakening the contract. The interrupted second generation job and runtime environment/cache mismatch are preserved in the implementation report. No prior helper demo is part of this change.
