# Corrected live caller measurements

The primary evidence below is routing-consistent full `fan_out_scoped`, not a raw kernel or network/server measurement. Every ordinary event has exactly one lower-case `h` matching its stored channel. The held-out cases put it FIRST, as the message client emits it, with `e`/`p`/`t` distractors and varied canonical UUIDv4 selectors. Selector ranks are chosen after canonical `BTreeSet` sorting. All complete recipient identities and subscription IDs were checked outside timing.

Intel Xeon 6975P-C on Blox, 32 affinity CPUs with an 8-CPU-equivalent quota. Historical, immutable first verified generated source and selected final use the same normal Hermit Rust 1.95.0 release compiler and standalone lock. The exact current registry owner is shared; the research-only first-generated build substitutes only its matcher/projection import. Preparation and fixture construction are outside fanout timing; index/current-entry/scope checks, complete predicate, one lazy event projection, deduplication and result construction are inside. Registration/replacement is timed separately. These configurations and optimizer weights are synthetic, not measured traffic.

## Eight independent held-out configurations

These were measured AFTER optimization and excluded from both frozen objectives. `middle` is the fixed sorted middle rank; `seeded16` is a prebuilt fixed-seed sequence of 16 interior ranks, with time divided by 16 to report per-fanout cost. No rank sequence was constructed inside timing.

| Subscriptions | Selectors | Rank selection | Historical µs | First µs | Selected final µs | Historical / final | First / final |
| ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 50 | middle | 11.526 | 0.963 | 0.957 | 12.05× | 1.01× |
| 1 | 50 | seeded16 | 10.921 | 1.021 | 0.991 | 11.02× | 1.03× |
| 100 | 50 | middle | 1158.131 | 26.781 | 22.557 | 51.34× | 1.19× |
| 100 | 50 | seeded16 | 1089.509 | 27.880 | 24.149 | 45.12× | 1.15× |
| 1 | 128 | middle | 28.973 | 1.039 | 1.012 | 28.63× | 1.03× |
| 1 | 128 | seeded16 | 24.316 | 1.081 | 1.042 | 23.33× | 1.04× |
| 100 | 128 | middle | 2925.465 | 34.147 | 29.775 | 98.25× | 1.15× |
| 100 | 128 | seeded16 | 2446.606 | 36.489 | 32.028 | 76.39× | 1.14× |

The generated matcher generalizes to these interior-rank cases: historical/final ranges from about 11× to 98×. Most of that gain was already present in the first generated implementation plus the approved preparation lifetime. The selected optimizer adds a further 1.14–1.19× speedup for the 100-subscription held-out cases; tiny single-subscription first/final differences are not a headline. This is not an unconditional speedup.

## Corrected first/last matrix and index controls

First/last is sorted selector rank; early means the sole `h` is first, and late means it is last with NO preceding `h`. Late rows are valid ordering stress, not the primary client shape. Miss means an off-scope channel and correctly has zero candidates. Full 76-case raw matrices retain 1/10/100/1000 subscriptions, 1/50/128 selectors, early/late/first/last, off-scope, other-community, wrong-kind and global-p cases.

| Case | Historical µs | First µs | Selected final µs | Historical / final |
| --- | ---: | ---: | ---: | ---: |
| live_fanout1_h1_first_early | 0.245 | 0.926 | 0.933 | 0.26× |
| live_fanout1_h128_first_early | 0.244 | 1.037 | 0.938 | 0.26× |
| live_fanout10_h50_first_early | 2.300 | 3.756 | 2.631 | 0.87× |
| live_fanout100_h128_first_early | 24.821 | 35.140 | 19.370 | 1.28× |
| live_fanout1000_h1_first_early | 231.240 | 216.194 | 172.214 | 1.34× |
| live_fanout1000_h50_first_early | 273.568 | 522.478 | 221.007 | 1.24× |
| live_fanout1000_h128_first_early | 304.098 | 791.615 | 266.983 | 1.14× |
| live_fanout1000_h50_last_early | 22888.726 | 435.241 | 409.545 | 55.89× |
| live_fanout1000_h128_last_early | 58422.615 | 594.156 | 594.242 | 98.31× |
| live_fanout1000_h128_last_late | 60130.626 | 588.646 | 589.938 | 101.93× |
| live_fanout1000_h128_last_miss | 0.047 | 0.046 | 0.046 | 1.03× |
| fanout1000_zero_candidates | 0.045 | 0.046 | 0.046 | 0.98× |
| fanout1000_wrong_kind | 0.045 | 0.046 | 0.046 | 0.98× |
| fanout1000_other_community | 0.045 | 0.046 | 0.047 | 0.96× |
| fanout1000_global_p_index | 227.579 | 230.798 | 180.592 | 1.26× |

Single-subscription first-rank cases remain about 3.8× slower (~0.7 µs extra) because event projection is not amortized. The first generated 1,000-subscription/128-selector first-rank case is also a material regression (304 → 792 µs), though selected final reduces it to 267 µs. The 10-subscription/50-selector first-rank case also remains ~14% slower (2.30 → 2.63 µs). No production threshold/fallback or objective retuning was added. Existing indices eliminate zero-candidate/off-scope/wrong-kind/other-community controls before event projection; these are not scans of every registered filter. The old per-call adapter prototype remains rejected and undeployed (old kind miss ~7.7 ns versus ~738 ns with fresh selector/event projection).

## Actual implement and optimize results

`cargo djinn implement` generated the complete matcher/proofs. The original three real optimizer cycles accepted two and verified/rejected/reverted the third; those genuine outcomes remain [archived](STRUCTURAL-V1.md). Their late/miss fixtures manually stored a later selected UUID despite an earlier parseable unrelated h, so they are structural stress only, NOT ordinary live-fanout headlines. The prior middle-tag holdout had the same defect. This was a fixture error, not a production bug or failed proof. Original early rows were routing-consistent, including recorded first-source regressions.

After the owner move and fixture correction, a genuine extra `optimize --cycles 1 --jobs 1 --min-time 5 --batches 9 --threshold 3` ran against a frozen corrected workload. It verified a concrete combined-scan/suffix/inlining candidate but rejected/reverted it: baseline 5.838 ms (3.5% spread), candidate 5.898 ms (1.0% spread), −1.0%, needed 3.5%. No extra improvement is claimed. The selected source remains the prior accepted SHA `c4ddd3602eb5c65e9e04ac639f68cd3d7fb34b468b28f61775638579cab509bd`. Exact [driver outcomes](corrected-optimize-outcomes.json) and rejected checkpoint are preserved.

Final scoring verifies/syncs the jail into original source, then benchmarks original config after the child monitor has joined. A 1Hz sampler recorded eight final scoring snapshots without concurrent Lean/Lake; final spread was 1.0%. Child exploratory timing requires verification sync after each edit, can contend with periodic Lean type-checking, and was not used to accept candidates. See [tool/process audit](TOOLCHAIN.md).

## Registration and memory costs

Same-ID registration/replacement includes caller-side input cloning, removal, authoritative replacement and indexing; selector preparation is additional in generated builds.

| Registration case | Historical µs | First µs | Selected final µs |
| --- | ---: | ---: | ---: |
| live_fanout1_h1_first_late | 0.803 | 0.924 | 0.930 |
| live_fanout1_h50_first_late | 30.644 | 31.947 | 32.060 |
| live_fanout1_h128_first_late | 88.400 | 90.921 | 90.425 |

Additional retained Vec capacities plus the new entry header: 274 / 3,214 / 7,894 bytes for 1 / 50 / 128 selectors, with 6 / 55 / 133 additional allocations. This excludes allocator metadata/bin rounding; 1,000 subscriptions with 128 selectors add ~7.9 MB. Event projection is transient and included in fanout timings. Raw accounting is [live-prepared-memory.json](live-prepared-memory.json).

## Raw samples, provenance and limits

Warmup is 100 iterations, calibration targets at least 40 ms per batch, nine serial raw batches per row, inputs/results `black_box`ed and outputs consumed. Raw JSON retains iteration counts, batch durations, min/median/max; no flaky performance CI threshold exists. Metadata binds the exact source/driver/archive hashes and binary hashes, compiler, CPU/quota and measurement sequence: [corrected-benchmark-metadata.json](corrected-benchmark-metadata.json).

| Selected final case | Spread (max−min)/median | Batch duration seconds |
| --- | ---: | --- |
| heldout_n1_h50_middle | 0.92% | 0.062–0.063 |
| heldout_n100_h50_seeded16 | 1.51% | 0.049–0.050 |
| heldout_n100_h128_middle | 0.75% | 0.061–0.061 |
| heldout_n100_h128_seeded16 | 1.75% | 0.065–0.067 |

Raw corrected files: `live-{historical,first,final}-fanout.json` (including registration) and `live-{historical,first,final}-holdout.json`. The earlier `first-predicate`/`final-predicate` files isolate prepared predicate/projection and are NOT full caller timings. Old raw matrices/checkpoints retain their original source metadata and are explicitly superseded for product-facing late/miss claims.

Blox locked-workspace resolution remains mirror-blocked; these same-lock standalone builds use UUID/serde_json versions recorded in dependency-versions.json. The isolated macOS clone separately passed normal locked core filter tests (11/11), all relay subscription tests (35/35), core+relay all-target Clippy with warnings denied, workspace formatting and diff checks on the retained `c4ddd360…509bd` source. Its eight runtime hashes match this export; [validation provenance](parent-validation-provenance.json) distinguishes this result from the earlier in-progress candidate and compilation-only hook preparation. This focused record does not establish the complete push-hook suites. No local relay/HTTP/WebSocket endpoint workflow, network/server throughput, app-wide gain, other-platform acceptance or human acceptance is claimed. No historical correctness bug was found. The formal proof covers the complete structural predicate, not registry/auth/concurrency or its data adapter.
