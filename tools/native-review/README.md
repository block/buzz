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

## MVP limits

Only macOS desktop, the local review-channel fixture, role/name/identifier AX
locators, click/hover/key/wait actions, window screenshots/video, and step AX
snapshots are implemented. iOS Simulator and repeated base/head performance
comparison remain later phases.
