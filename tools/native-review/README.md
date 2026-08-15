# Buzz native review harness

This macOS-only MVP drives the real Tauri/WKWebView app with Accessibility and
CGEvent, captures its window with Core Graphics and AVFoundation, and writes an
exact-SHA run receipt. It is a targeted review lane, not a replacement for
`just ci` or Playwright.

## Safety contract

- Only loopback `ws://`/`http://` relays are accepted.
- Every run gets an ephemeral Nostr key, run-specific dev bundle ID, keyring
  service, HOME, WebKit/app-data state, and artifact directory.
- The launcher environment is allowlisted before the ephemeral key is added;
  inherited tokens and production keys never enter the reviewed process.
- Production bundle IDs, keyring services, and remote relays fail closed.
- This protects reviewer state from accidents. It is **not** containment for
  hostile code; use a dedicated macOS user or disposable VM for untrusted PRs.

## Commands

```bash
just native-review-doctor
just native-review-desktop tools/native-review/desktop/tooltip-fresh-dwell.yaml
python3 -m unittest discover -s tools/native-review/tests -p 'test_*.py'
```

The desktop command expects the isolated `buzz-harness` relay on port 3030
(`scripts/start-isolated-test-relay.sh`). Doctor reports Accessibility and
Screen Recording separately and the run refuses to proceed unless both are
already granted to the invoking terminal/agent.

Runs are written under
`test-results/native-review/<sha>/<flow>/<run-id>/`. A failed locator,
postcondition, recording, evidence capture, or cleanup produces a failed partial
receipt. `tests/fixtures/broken-tooltip.yaml` is the deliberate fail-loud
mutation.

## Performance comparison and budgets

A step with `measure: <name>` persists its complete native action-to-observed-
postcondition duration in the receipt. While the journey runs, the harness also
samples the app process every 100 ms and records median/peak CPU percentage and
resident memory. Capture a cohort rather than trusting one noisy laptop run:

```bash
just native-review-benchmark tools/native-review/desktop/tooltip-fresh-dwell.yaml 5
```

Compare at least three clean baseline receipts with at least three clean
candidate receipts using `compare` and a checked-in policy such as
`performance/tooltip-fresh-dwell.yaml`:

```bash
./tools/native-review/bin/review-native compare \
  --baseline /path/base-1/receipt.json --baseline /path/base-2/receipt.json --baseline /path/base-3/receipt.json \
  --candidate /path/head-1/receipt.json --candidate /path/head-2/receipt.json --candidate /path/head-3/receipt.json \
  --budget tools/native-review/performance/tooltip-fresh-dwell.yaml \
  --output test-results/native-review/performance-comparison.json
```

Comparison uses cohort medians, reports every raw sample and min/max, and exits
nonzero when an absolute ceiling or relative regression limit is breached. It
fails closed for dirty-tree runs, failed cleanup, mixed source revisions within a
cohort, wrong flows, missing metrics, too few samples, or different machine/OS
fingerprints. Baseline and candidate therefore need to run on the same host;
thermal/load noise is reduced by repeated samples, not disguised as universal
lab-grade benchmarking. Recording overhead is intentionally present in both
cohorts because this tool measures the reviewer-visible workflow.

## Current limits

macOS desktop and the local review-channel fixture are implemented. The schema
covers role/name/identifier locators; native click, hover, text entry, keyboard
shortcuts, scrolling, and waits; value/focus/existence assertions; window
screenshots/video; and per-step semantic + AX snapshots. iOS Simulator remains
a later phase.
