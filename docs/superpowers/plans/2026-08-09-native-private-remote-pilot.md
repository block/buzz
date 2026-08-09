# Native Private Remote Pilot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pair the native Buzz iPhone client with MacBook-hosted Command Adviser through a tailnet-private WSS relay, with no public ingress or hosted fallback and with actionable VPN/authentication connection state.

**Architecture:** Tailscale Serve terminates TLS at the MacBook's stable `*.ts.net` name and forwards only within the tailnet to the existing local relay. Desktop pairing gains an optional, persisted mobile relay origin which is independently validated in Rust and used only in the NIP-AB QR/payload; it does not change the desktop workspace or agent relay. The mobile client keeps its existing production WSS policy and adds truthful failure classification and visible recovery copy.

**Tech Stack:** Tauri 2/Rust, React 19/TypeScript, Flutter/Riverpod/Dart, NIP-AB/NIP-42, Tailscale Serve, Playwright, Rust and Flutter tests.

## Global Constraints

- The MacBook remains authoritative for relay, managed agents, models, RAG, Memory, signed events, and application data.
- Tailscale Funnel, public ingress, hosted relays, and cloud copies of Command Adviser data remain disabled and out of scope.
- The pilot supports only a validated `https://<name>.ts.net` advertised origin; the derived pairing URL is `wss://<name>.ts.net`.
- The ordinary desktop workspace relay stays unchanged; the advertised address is pairing-only.
- Existing NIP-AB QR/SAS, owner identity, NIP-42 authentication, mobile community storage, and signed messaging remain authoritative.
- APNs, background wake, durable outbox, device attestation, and always-on hosting are deferred.
- No production code is written before its failing behavior test has been observed.

---

### Task 1: Trusted desktop pairing advertisement

**Files:**
- Modify: `desktop/src-tauri/src/commands/pairing.rs`
- Modify: `desktop/src-tauri/src/commands/pairing_relay_tests.rs`
- Modify: `desktop/src/shared/api/tauri.ts`

**Interfaces:**
- Consumes: current workspace relay from `relay_ws_url_with_override` and `relay_api_base_url_with_override`.
- Produces: `startPairing(advertisedRelayUrl?: string): Promise<string>` and Rust `resolve_advertised_mobile_relay(Option<&str>, default_ws, default_http) -> Result<AdvertisedMobileRelay, String>`.

- [x] **Step 1: Write failing Rust tests for tailnet-only normalization**

Add table-driven tests proving:

```rust
let resolved = resolve_advertised_mobile_relay(
    Some("https://matthews-macbook-pro-1.tailf29f2c.ts.net"),
    "ws://localhost:3000",
    "http://localhost:3000",
)?;
assert_eq!(resolved.ws_url, "wss://matthews-macbook-pro-1.tailf29f2c.ts.net/");
assert_eq!(resolved.http_url, "https://matthews-macbook-pro-1.tailf29f2c.ts.net/");
assert!(resolved.is_private_tailnet);
```

Reject `http://`, non-`ts.net` hosts, credentials, query, fragment, non-root path, and empty host. Prove `None` preserves the current relay unchanged.

- [x] **Step 2: Run the Rust test and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml pairing_relay -- --nocapture
```

Expected: compilation fails because `resolve_advertised_mobile_relay` does not exist.

- [x] **Step 3: Implement the smallest server-trusted resolver**

Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct AdvertisedMobileRelay {
    ws_url: String,
    http_url: String,
    is_private_tailnet: bool,
}
```

Parse with `url::Url`, require an HTTPS origin whose lowercase host ends in `.ts.net`, normalize the path to `/`, and derive WSS by changing only the scheme. When no override is supplied, retain the current main-relay URLs and set `is_private_tailnet` false.

- [x] **Step 4: Thread the optional origin through NIP-AB pairing**

Change the Tauri command to:

```rust
pub async fn start_pairing(
    advertised_relay_url: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
    pairing: State<'_, PairingHandle>,
) -> Result<String, String>
```

Use the resolved WSS URL for pairing-relay discovery and the QR. Use the resolved HTTPS URL in the encrypted identity payload. Append `&transport=tailnet` only for the private override so the code is auditable without changing the NIP-AB cryptographic transcript.

- [x] **Step 5: Update the desktop API and verify GREEN**

Implement:

```ts
export async function startPairing(
  advertisedRelayUrl?: string,
): Promise<string> {
  return invokeTauri<string>("start_pairing", {
    advertisedRelayUrl: advertisedRelayUrl || null,
  });
}
```

Run the focused Rust tests again and require PASS.

- [x] **Step 6: Commit Task 1**

```bash
git add desktop/src-tauri/src/commands/pairing.rs \
  desktop/src-tauri/src/commands/pairing_relay_tests.rs \
  desktop/src/shared/api/tauri.ts
git commit -m "feat: advertise a private mobile relay for pairing"
```

### Task 2: Persisted private-iPhone relay setting

**Files:**
- Create: `desktop/src/features/settings/lib/privateMobileRelay.ts`
- Create: `desktop/src/features/settings/lib/privateMobileRelay.test.mjs`
- Modify: `desktop/src/features/settings/ui/MobilePairingCard.tsx`
- Modify: `desktop/tests/e2e/mobile-pairing-qr.spec.ts`
- Modify: `desktop/src/testing/e2eBridge.ts`

**Interfaces:**
- Consumes: `startPairing(advertisedRelayUrl?: string)` from Task 1.
- Produces: `normalizePrivateMobileRelay(raw: string): { value: string; error: string | null }`, storage key `command-adviser.private-mobile-relay.v1`, and a separate Settings card that does not alter existing QR layout geometry.

- [x] **Step 1: Write failing URL-helper tests**

Test literal outcomes:

```js
assert.deepEqual(
  normalizePrivateMobileRelay("matthews-macbook-pro-1.tailf29f2c.ts.net"),
  {
    value: "https://matthews-macbook-pro-1.tailf29f2c.ts.net/",
    error: null,
  },
);
```

Also cover a complete HTTPS input, surrounding whitespace, blank-as-disabled, and rejection of HTTP, public domains, credentials, paths, query strings, and fragments.

- [x] **Step 2: Run the helper test and verify RED**

Run:

```bash
cd desktop
pnpm test -- src/features/settings/lib/privateMobileRelay.test.mjs
```

Expected: module-not-found failure.

- [x] **Step 3: Implement normalization and safe persistence**

Keep the helper pure. In `MobilePairingCard`, initialize from localStorage, validate on change, and persist only a valid normalized origin or remove the key when blank. A storage exception must leave the in-memory value usable and must not block ordinary local pairing.

- [x] **Step 4: Add the separate private-relay Settings row**

Render a `SettingsOptionGroup` before the unchanged QR group with:

- label `Private iPhone relay`;
- input placeholder `https://your-mac.tailnet.ts.net`;
- copy explaining that Tailscale must be active and the setting affects mobile pairing only; and
- inline validation text.

Call `startPairing(normalized.value || undefined)` and disable Start only when a non-empty value is invalid.

- [x] **Step 5: Write and run the Playwright behavior test**

Seed the input with `https://matthews-macbook-pro-1.tailf29f2c.ts.net`, start pairing, and assert the real Tauri mock command log contains:

```ts
{
  command: "start_pairing",
  args: {
    advertisedRelayUrl:
      "https://matthews-macbook-pro-1.tailf29f2c.ts.net/",
  },
}
```

Reload Settings and prove the value persists. First run must fail because the field and argument do not exist; after implementation the focused Playwright project must pass.

- [x] **Step 6: Run desktop focused gates and commit Task 2**

```bash
cd desktop
pnpm test -- src/features/settings/lib/privateMobileRelay.test.mjs
pnpm exec playwright test tests/e2e/mobile-pairing-qr.spec.ts --project=smoke
cd ..
git add desktop/src/features/settings/lib/privateMobileRelay.ts \
  desktop/src/features/settings/lib/privateMobileRelay.test.mjs \
  desktop/src/features/settings/ui/MobilePairingCard.tsx \
  desktop/tests/e2e/mobile-pairing-qr.spec.ts \
  desktop/src/testing/e2eBridge.ts
git commit -m "feat: configure private iPhone relay pairing"
```

### Task 3: Truthful mobile VPN and authentication diagnostics

**Files:**
- Modify: `mobile/lib/shared/relay/relay_session.dart`
- Create: `mobile/lib/shared/relay/connection_status.dart`
- Modify: `mobile/lib/shared/relay/relay.dart`
- Modify: `mobile/lib/features/settings/settings_page/connection_section.dart`
- Modify: `mobile/lib/features/channels/channels_page.dart`
- Modify: `mobile/lib/features/channels/channels_page/body.dart`
- Modify: `mobile/lib/features/channels/channels_page/skeleton.dart`
- Modify: `mobile/test/shared/relay/relay_session_test.dart`
- Create: `mobile/test/shared/relay/connection_status_test.dart`
- Modify: `mobile/test/features/channels/channels_page_test.dart`

**Interfaces:**
- Consumes: existing `RelayAuthRejectedException`, `RelayConfig.baseUrl`, and `SessionStatus`.
- Produces: `SessionFailureKind { network, authentication }`, optional `SessionState.failureKind`, `isTailnetRelayUrl(String)`, and `relayConnectionPresentation(String, SessionState)`.

- [x] **Step 1: Write failing relay-state tests**

Extend the real session tests to assert:

```dart
expect(session.state.failureKind, SessionFailureKind.authentication);
```

after an auth rejection, and `SessionFailureKind.network` while reconnecting after a socket/network failure. Confirm successful connection clears the failure.

- [x] **Step 2: Run the relay tests and verify RED**

Run:

```bash
cd mobile
flutter test test/shared/relay/relay_session_test.dart
```

Expected: compilation fails because `failureKind` and its enum do not exist.

- [x] **Step 3: Implement failure classification**

Add the enum and immutable field. `_handleDisconnected` sets authentication for `RelayAuthRejectedException`; `_scheduleReconnect` sets network. `_handleConnected` clears it. No branch signs out, removes a community, or selects another relay.

- [x] **Step 4: Write failing presentation tests**

Test literal presentations for:

- a connected `https://...ts.net` community;
- tailnet reconnect/network failure: `Private relay unavailable` plus `Check Tailscale or VPN`;
- authentication rejection: `Authentication failed` plus re-pair guidance; and
- an ordinary public relay reconnect, which must not mention Tailscale.

- [x] **Step 5: Implement and surface the presentation**

Create a shared immutable presentation record and use it in Settings. In the delayed reconnect skeleton, render the title and detail above the shape placeholders, passing the active relay URL and full `SessionState` rather than only `SessionStatus`. Keep the existing skeleton geometry and accessibility live region.

- [x] **Step 6: Add the screen behavior test and verify GREEN**

Override the active community with a `.ts.net` relay and the session with a network failure. After the existing two-second delay, assert visible `Private relay unavailable` and `Check Tailscale or VPN`. Then switch the session to connected and prove channel content returns.

Run:

```bash
cd mobile
flutter test test/shared/relay/relay_session_test.dart
flutter test test/shared/relay/connection_status_test.dart
flutter test test/features/channels/channels_page_test.dart
```

- [x] **Step 7: Commit Task 3**

```bash
git add mobile/lib mobile/test
git commit -m "feat: explain private relay connection failures"
```

### Task 4: Tailnet configuration, release gates, and physical-device handoff

**Files:**
- Create: `docs/testing/native-private-remote-pilot.md`
- Modify: `docs/command-console/ROADMAP.md`
- Modify: `docs/superpowers/specs/2026-08-09-native-private-remote-access-v0.5.8.md`

**Interfaces:**
- Consumes: the validated desktop pairing setting and mobile failure presentation.
- Produces: a repeatable Tailscale Serve and iPhone acceptance runbook with rollback.

- [x] **Step 1: Configure tailnet-private WSS and verify the relay**

Run with the bundled CLI:

```bash
TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale serve --bg --yes http://127.0.0.1:3000
TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale serve status --json
curl --fail --header 'Accept: application/nostr+json' \
  https://matthews-macbook-pro-1.tailf29f2c.ts.net/
```

Require a tailnet-only HTTPS endpoint returning the existing Buzz NIP-11 document. Record rollback as `tailscale serve reset`. Confirm Funnel remains absent.

- [x] **Step 2: Write the acceptance runbook**

Document exact prerequisites, the advertised URL, QR/SAS pairing, signed DM/history/Command Adviser turn, Tailscale-off fail-closed check, reconnect/no-duplicate check, existing-community regression, and rollback. Clearly mark physical iPhone actions as the only user gate.

- [x] **Step 3: Run the full repository gate**

```bash
. ./bin/activate-hermit
just ci
```

Also run the mobile safe gates explicitly:

```bash
cd mobile
dart format --output=none --set-exit-if-changed .
flutter analyze
flutter test
```

- [x] **Step 4: Build and verify installable candidates**

Build the signed macOS release candidate with `just desktop-release-build aarch64-apple-darwin`. Build/test the native iOS runner only through the repository's supported non-destructive test path; do not invoke forbidden `flutter build` or `flutter run` commands.

- [ ] **Step 5: Update roadmap/specification and record the checkpoint**

Roadmap and specification updates are complete. The Memory MCP write remains
pending because the configured LAN endpoint was unreachable at handoff.

Mark automated pilot implementation complete and list the physical-device journey as the remaining gate. Record the architecture, Tailscale Serve configuration, tests, and rollback in Memory MCP with agent `CODEX`.

- [ ] **Step 6: Commit, push, and stop at the iPhone gate**

```bash
git add docs/testing/native-private-remote-pilot.md \
  docs/command-console/ROADMAP.md \
  docs/superpowers/specs/2026-08-09-native-private-remote-access-v0.5.8.md
git commit -m "docs: add private remote pilot acceptance runbook"
git push origin codex/phase-native-private-remote-pilot
```

At this point request only the physical iPhone actions: install/open the candidate, confirm Tailscale is connected, scan the QR, compare SAS, switch the phone to mobile data, and perform the acceptance messages. Do not claim end-to-end remote acceptance before those actions pass.

## Self-review record

- Spec coverage: relay bind/advertise, production-safe private WSS, pairing, visible VPN/auth failure, no hosted fallback, and repeatable acceptance are each assigned to a task.
- Deferred-scope check: APNs, wake, outbox, public ingress, model relocation, RAG/Memory replication, and always-on hosting do not appear as implementation work.
- Type consistency: the desktop uses one normalized HTTPS advertised origin; Rust derives WSS. Mobile diagnostics carry `SessionFailureKind` in `SessionState` and render through one shared presentation function.
- Placeholder scan: no TBD/TODO or unspecified implementation step remains.
