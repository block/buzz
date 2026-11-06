# Archived structural training evidence — not ordinary live routing

The original frozen objective and raw results are preserved unchanged below. Its late/miss h fixtures contained an earlier unrelated parseable h, while manually assigning the stored channel to a later selected UUID. Production ingest routes to the FIRST parseable h. Those late/miss rows, and the old middle-tag holdout, therefore do NOT represent ordinary channel live fanout; they are structural stress evidence only. This fixture defect is neither a production correctness bug nor a proof failure. Early rows were routing-consistent; their regressions remain relevant. Corrected primary live and held-out measurements are in [MEASUREMENTS.md](MEASUREMENTS.md). The original three real optimization decisions and proof snapshots remain genuine, with that objective limitation.

# Full caller measurements

Intel Xeon 6975P-C on this Blox; 32 affinity CPUs with an 8-CPU-equivalent cgroup quota. Normal Hermit Rust 1.95.0 release builds, same standalone lock, warmup and nine raw batches. Synthetic cases are not a measured traffic mix. `hN` is the number of canonical channel selectors; event tags contain mixed keys. First/last denotes selected value position, early/late denotes matching tag position. All output identities/counts were checked before timing.

The boundary is **actual `fan_out_scoped`**, including index lookup, current subscription lookup and community/scope guards, complete predicate, one lazy event projection where deployed, deduplication and result construction. It excludes network, later membership/access filtering and databases. Registration preparation is measured separately. Root workspace compilation was blocked; harness UUID/serde_json versions differ from the workspace lock as recorded in [dependency-versions.json](dependency-versions.json).

| Case | Historical µs | First generated µs | Final µs | Historical / final |
| --- | ---: | ---: | ---: | ---: |
| fanout1_h1_first_early | 0.237 | 0.950 | 0.913 | 0.26× |
| fanout10_h1_first_late | 6.689 | 2.249 | 2.173 | 3.08× |
| fanout100_h1_first_early | 23.115 | 20.613 | 17.483 | 1.32× |
| fanout1000_h1_first_early | 231.086 | 202.982 | 172.602 | 1.34× |
| fanout1000_h50_first_early | 262.092 | 719.882 | 224.666 | 1.17× |
| fanout1000_h50_last_late | 25134.773 | 446.929 | 406.616 | 61.81× |
| fanout1000_h128_first_early | 304.429 | 1218.763 | 257.315 | 1.18× |
| fanout1000_h128_last_late | 63523.553 | 616.958 | 584.161 | 108.74× |
| fanout1000_h128_last_miss | 62146.897 | 1109.975 | 930.246 | 66.81× |
| fanout1000_global_p_index | 227.474 | 228.430 | 181.599 | 1.25× |
| fanout1000_zero_candidates | 0.045 | 0.046 | 0.046 | 0.98× |

The single-subscription early case is **3.85× slower**, an additional ~0.68 µs per fanout, because event projection cannot be amortized across candidates. The rejected per-call prototype is not deployed: kind miss was ~7.7 ns historically versus ~738 ns with fresh selector/event projection. Prepared live deployment improves high-fanout and repeated selector scans, but these results do not justify a universal speedup claim. Zero-candidate/wrong-kind/other-community cases are mostly eliminated by existing indices and avoid event projection; they are not full scans of all registered filters.

An independent middle-position holdout was excluded from the optimization objective:

| Holdout case | Historical µs | Final µs | Historical / final |
| --- | ---: | ---: | ---: |
| holdout_fanout1_h1_middle | 0.525 | 0.940 | 0.56× |
| holdout_fanout10_h50_middle | 131.475 | 3.364 | 39.08× |
| holdout_fanout1000_h128_middle | 32845.797 | 925.754 | 35.48× |

## Actual generation and optimization

`implement` generated the first complete solution/proofs; no matching Rust or Lean was hand-authored by the outer owner. The real optimizer used the frozen actual caller workload with `--cycles 3 --jobs 1 --min-time 5 --batches 9 --threshold 3` on isolated Rust nightly-2026-09-17. Its synthetic equal-case objective, not a traffic-weighted metric, gave:

| Driver round | Median ms/iteration | Decision | Batch spread |
| --- | ---: | --- | ---: |
| First verified baseline | 10.297 | baseline | 2.3% |
| 1 | 8.019 | accepted, 22.1% better | 3.9% |
| 2 | 7.560 | accepted, 5.7% better | 2.1% |
| 3 | 7.576 | verified but not faster; reverted | 1.2% |

The retained candidate is round 2: 1.36× faster than the first generated implementation under that objective. Exploratory child measurements had substantial noise and are not substituted for driver decisions. Slice equality alone and grouped byte comparisons were measured and discarded inside earlier rounds. Every accepted round passed 16/16 proof obligations and the standard axiom audit. The rejected third round also passed proof; its timing did not meet the improvement threshold. The [structured outcomes](optimize-outcomes.json) and immutable generation checkpoints retain provenance.

## Costs and dispersion

| Same-ID registration/replacement, input clone included | Historical µs | First µs | Final µs |
| --- | ---: | ---: | ---: |
| fanout1_h1_first_late | 0.777 | 0.911 | 0.900 |
| fanout1_h50_first_late | 29.283 | 31.194 | 31.224 |
| fanout1_h128_first_late | 81.944 | 89.447 | 87.088 |

Retained additional prepared capacity is 274 / 3,214 / 7,894 bytes per subscription for 1 / 50 / 128 channels, with 6 / 55 / 133 additional allocations. This counts actual Vec capacities and the new entry header, excluding allocator metadata/bin rounding. 1,000 subscriptions with 128 selectors add about 7.9 MB by that accounting. Event projection is transient, separately timed inside fanout.

All raw samples, iteration counts, batch durations and min/median/max are retained in the JSON files. Final selected caller case ranges (max-minus-min as a percentage of median) are:

- fanout1_h1_first_early: 0.66%, batches 0.060–0.060 s, 65536 calls/batch.
- fanout1000_h128_first_early: 1.65%, batches 0.066–0.067 s, 256 calls/batch.
- fanout1000_h128_last_late: 0.77%, batches 0.075–0.075 s, 128 calls/batch.
- fanout1000_global_p_index: 0.46%, batches 0.046–0.047 s, 256 calls/batch.

`first-predicate.json` / `final-predicate.json` separate the full prepared kernel from the full predicate plus event projection; those are **not end-to-end caller timings**. Prototype JSON files retain the rejected per-call adapter comparison. Exact source/CPU/compiler metadata is in the first/final metadata JSON. Formatting after optimization changed only layout/import ordering; specification tokens were checked identical to the first verified report, followed by a fresh complete verification and separate axiom audit. The immutable first wrapper snapshot predates the layout-only compatibility alias retention; that difference is recorded in the freeze metadata.

No historical correctness bug was observed. The generated solutions satisfy the complete declarative structural contract; the earlier nested-closure translation scope failure is a tool/spec-plumbing limitation, not a discovered Buzz bug. No deliberate mutant is presented as historical evidence.

The first and cycle-one proof checkpoints are stored as `first/djinn/proofs/RelayFilter/Proofs.lean.gz` and `cycle1/djinn/proofs/RelayFilter/Proofs.lean.gz` to preserve their exact generated bytes (including original whitespace); decompress them when restoring those checkpoints. Hash entries named `Proofs.lean` refer to the decompressed source; the cycle-one `Proofs.lean.gz` entry records the deterministic gzip container. The final active proof is ordinary readable Lean source.

## Additional routing-consistent early regressions retained

| Original case | Historical µs | First µs | Selected final µs |
| --- | ---: | ---: | ---: |
| fanout1_h1_first_early | 0.237 | 0.950 | 0.913 |
| fanout1_h128_first_early | 0.239 | 1.471 | 0.916 |
| fanout1000_h128_first_early | 304.429 | 1218.763 | 257.315 |
