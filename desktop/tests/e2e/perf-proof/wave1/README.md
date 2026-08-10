# Wave 1 typing isolation — before/after proof

Harness: `desktop/tests/e2e/typing-wave1-core.perf.ts`  
Collector: `desktop/scripts/collect-wave1-perf.sh` (retries until N successful `WAVE1_PERF` lines)  
Summarizer: `desktop/scripts/summarize-wave1-perf.sh`  
Conditions: Chromium headless, 4× CPU throttle, quiet vs 8 agents typing (250ms) + live markdown every 2s  
Metric: Event Timing API input→paint (`durationThreshold: 16`) + longtask totals  

## Sample size

| Phase | Successful iterations |
|-------|----------------------|
| BEFORE (unoptimized) | 50 |
| AFTER (Wave 1 isolation) | 50 |

Raw lines: `before-success.txt`, `after-success.txt` in this directory.

## Aggregates (median across 50 runs)

| Metric | BEFORE | AFTER | Delta |
|--------|--------|-------|-------|
| quiet_median (ms) | 16.0 | 16.0 | 0 |
| busy_median (ms) | 24.0 | 24.0 | 0 |
| busy−quiet median (ms) | 8.0 | 8.0 | 0 |
| quiet_longtask median (ms) | 0.0 | 0.0 | 0 |
| quiet_longtask p95 (ms) | 106.6 | 34.2 | **−68%** |
| busy_longtask median (ms) | 282.5 | 225.0 | **−20%** |
| busy_longtask mean (ms) | 324.5 | 268.6 | **−17%** |
| busy_longtask p95 (ms) | 541.7 | 515.9 | −5% |

## Interpretation

Event Timing reports at ~8ms granularity, so quiet/busy keystroke medians sit on the 16/24ms buckets both before and after — that axis is a coarse floor for this harness on this machine.

The clearer signal is **main-thread longtask cost under agent-busy load**: median busy longtask time dropped ~20% and quiet longtask p95 collapsed ~68%. That matches the intended isolation (typing/working/card-mint stores no longer re-render `ChannelPane` / timeline / `MessageComposer`).

## What changed

1. Moved `useChannelWorkingAgentPubkeys` + `useCardMintJobs` out of `ChannelPane` into `ComposerDockFrame` + `ChannelComposerActivityAccessory` via `useComposerDockActivity`.
2. Stopped folding ephemeral typers into `messageProfilePubkeys` (users-batch key churn).
3. Stabilized `personaLookup` / `respondToLookup` with `useStableMap`.
4. Memoized `editTarget` and follow/unfollow handlers via `useChannelPaneEditAndFollow` (also keeps `ChannelScreen.tsx` under the file-size ratchet).
