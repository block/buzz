# Wave 1b — timeline row memo boundaries — before/after proof

Harness: `desktop/tests/e2e/typing-wave1-core.perf.ts`  
Collector: `desktop/scripts/collect-wave1-perf.sh`  
Summarizer: `desktop/scripts/summarize-wave1-perf.sh`  
Baseline: Wave 1 tip (`perf/desktop-wave1-typing-isolation`)  
Conditions: Chromium headless, 4× CPU throttle, quiet vs 8 agents typing (250ms) + live markdown every 2s  

## Sample size

| Phase | Successful iterations |
|-------|----------------------|
| BEFORE (Wave 1 tip) | 10 |
| AFTER (row memo + thread panel memo) | 10 |

Raw lines: `before-success.txt`, `after-success.txt` in this directory.

## Aggregates (median across 10 runs)

| Metric | BEFORE | AFTER | Delta |
|--------|--------|-------|-------|
| quiet_median (ms) | 16.0 | 16.0 | 0 |
| busy_median (ms) | 28.0 | 28.0 | 0 |
| busy−quiet median (ms) | 12.0 | 12.0 | 0 |
| quiet_longtask median (ms) | 0.0 | 0.0 | 0 |
| **busy_longtask median (ms)** | **383.0** | **327.0** | **−15%** |
| busy_longtask mean (ms) | 403.7 | 360.5 | **−11%** |
| busy_longtask p95 (ms) | 568.9 | 593.0 | +4% |

## Interpretation

Event Timing keystroke medians stayed on the coarse 16/28ms floor. The clearer
signal is **busy-path main-thread longtask cost** while typing under live agent
spam: median −15%, mean −11%.

## What changed

1. `MessageRowItem` is `React.memo`'d with value equality on the timeline
   message + thread summary so live reformats skip unchanged *visible* rows
   (avoids rebuilding inline follow handlers / `MessageRow` props).
2. `MessageThreadPanel` is `React.memo`'d so pane-level re-renders with stable
   props skip the open-thread tree.
3. `MessageRow` short-circuits its custom comparator when `message` identity
   matches; thread head depth-normalization uses a WeakMap keyed on the source
   message (same pattern as inline reply normalization).
4. `timelineMessagesEqual` / `structureShareTimelineMessages` helpers live in
   `structureShareTimelineMessages.ts` for the row memo (format-path
   structure-sharing of every row was measured and rejected — it added O(n)
   equality cost on the main thread without beating React's visible-row path).
