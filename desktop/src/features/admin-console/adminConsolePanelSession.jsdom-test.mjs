/**
 * Session and settings behavior tests for AdminConsoleSettingsCard
 * and AdminConsolePanel. Covers origin-edit, same-session-save-race,
 * detail-navigation, blob-leak-on-back-navigation, cross-identity-delayed-save,
 * strict-mode-save, NIP-11 auto-discovery, and SettingsCard→panel wiring
 * (self-demotion, stale mutation, origin-switch, session teardown).
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  React,
  act,
  fireEvent,
  createRoot,
  QueryClient,
  QueryClientProvider,
  AdminConsoleSettingsCard,
  CommunitiesProvider,
  setIpcHandler,
  resetTestState,
  deferred,
  makeQueryClient,
  mountCard,
  mountPanel,
  settle,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";

afterEach(resetTestState);

// ── origin-edit ──────────────────────────────────────────────────────────────

test("origin-edit: input change while probe in-flight discards stale probe result", async () => {
  // Verifies that abortAndResetProbe() is wired to input onChange.
  //
  // Scenario:
  //  1. Component mounts with a saved origin; initial probe resolves
  //     immediately to "disabled" (no panel rendered, no unmocked IPC).
  //  2. User clicks Re-probe — new deferred probe starts.
  //  3. User edits the input via fireEvent.change — onChange fires, calls
  //     abortAndResetProbe(), setting probeAbortRef.current.signal.aborted.
  //  4. Stale probe resolves — the callback sees signal.aborted and returns
  //     early; probeUiState stays at { kind: "idle" } → panel never renders.
  //
  // Fails if abortAndResetProbe() is removed from the onChange handler:
  // the stale probe commits "nip98Authorized" and the panel renders.

  const pubkey = "d".repeat(64);
  const savedOrigin = "https://admin.example.com";

  setIpcHandler("get_admin_origin", () => Promise.resolve(savedOrigin));
  setIpcHandler("admin_probe", () => Promise.resolve({ state: "disabled" }));
  // If the stale probe commits nip98Authorized, the admin panel would render
  // and call these IPC commands. Mock them so the test doesn't hang.
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const qc = makeQueryClient(pubkey);
  const { container, doRender } = mountCard(qc);
  await doRender();
  await settle(25);

  // Re-probe button appears when savedOrigin is set.
  const reprobe = container.querySelector(
    "[data-testid='admin-probe-refresh']",
  );
  assert.ok(reprobe, "re-probe button must appear when savedOrigin is set");

  // Start a new deferred probe.
  const probeDeferred = deferred();
  setIpcHandler("admin_probe", () => probeDeferred.promise);

  await act(async () => {
    // fireEvent.click dispatches a native click — React's delegated onClick handler
    // calls runProbe(), creating a new AbortController on probeAbortRef.current.
    fireEvent.click(reprobe);
    await new Promise((r) => setTimeout(r, 5));
  });

  // Edit the input while the probe is in-flight. fireEvent.change dispatches
  // a native change event through React 19's container-level delegation,
  // reaching the production onChange handler which calls abortAndResetProbe().
  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(input, "origin input must be present");

  await act(async () => {
    fireEvent.change(input, {
      target: { value: "https://admin-new.example.com" },
    });
    await new Promise((r) => setTimeout(r, 5));
  });

  // Resolve the stale probe — controller.signal.aborted is true because
  // abortAndResetProbe() was called by onChange. The callback returns early.
  // We resolve inside act() so React flushes the state update synchronously.
  await act(async () => {
    probeDeferred.resolve({ state: "nip98Authorized" });
    await new Promise((r) => setTimeout(r, 20));
  });

  // The panel must NOT be visible — probeUiState is { kind: "idle" }, not
  // "authorized". The stale nip98Authorized result was discarded.
  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.ok(
    panel === null,
    "admin-console-panel must not render — stale probe discarded after onChange",
  );
  const text = container.textContent ?? "";
  assert.ok(
    !text.includes("Connected"),
    `stale nip98Authorized must not commit; got: ${text.slice(0, 200)}`,
  );

  // Skip unmount() here — calling act(root.unmount) after a mutation-caused
  // panel render would hang waiting for React cleanup. The assertions already
  // proved the test. The afterEach clears IPC handlers; the container is GC'd.
});

// ── same-session save race ────────────────────────────────────────────────────

test("same-session-save-race: deferred save X does not clobber pending save Y", async () => {
  // Verifies the sessionTokenRef fence in handleSave.
  //
  // The save button is disabled while isSaving=true. We use fireEvent.keyDown
  // with Enter on the input to trigger handleSave() directly (via onKeyDown),
  // bypassing the disabled save button. This lets both saves be in-flight
  // simultaneously — each with its own sessionToken.
  //
  // Scenario:
  //  1. Type X and press Enter — save X starts (deferred), token=X.
  //  2. Type Y and press Enter while X is pending — save Y starts (deferred),
  //     token=Y replaces X's token on sessionTokenRef.current.
  //  3. Resolve X late: token(X) != sessionTokenRef.current(Y) → returns early,
  //     no runProbe(originX).
  //  4. Resolve Y: runProbe(originY) fires normally.
  //
  // Fails if sessionTokenRef checks are removed: X's continuation calls
  // runProbe(originX) after Y has set its token, causing probeOrigins to
  // contain originX.

  const pubkey = "e".repeat(64);
  const originX = "https://admin-x.example.com";
  const originY = "https://admin-y.example.com";

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));

  let resolveX, resolveY;
  let saveCount = 0;
  setIpcHandler("set_admin_origin", () => {
    saveCount += 1;
    if (saveCount === 1)
      return new Promise((r) => {
        resolveX = r;
      });
    return new Promise((r) => {
      resolveY = r;
    });
  });

  // Track probe origins to detect if X erroneously fires a probe.
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({ state: "disabled" });
  });

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(15);

  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(input, "input must be present");

  // Type X and press Enter to start save X (deferred).
  await act(async () => {
    fireEvent.change(input, { target: { value: originX } });
    await new Promise((r) => setTimeout(r, 5));
  });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
    await new Promise((r) => setTimeout(r, 5));
  });

  // X's save is now pending (isSaving=true). Type Y and press Enter — this
  // calls handleSave() again despite isSaving=true, creating a new token(Y).
  await act(async () => {
    fireEvent.change(input, { target: { value: originY } });
    await new Promise((r) => setTimeout(r, 5));
  });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
    await new Promise((r) => setTimeout(r, 5));
  });

  // Both saves are now in-flight. Clear probes from any initial mount probes.
  probeOrigins.length = 0;

  // Resolve X late. Token(X) != sessionTokenRef.current (Y replaced it).
  // With token check: returns early, runProbe(originX) NOT called.
  // Without token check: runProbe(originX) IS called -> probeOrigins has originX.
  resolveX?.(originX);
  await settle(20);

  assert.ok(
    !probeOrigins.some((o) => o.includes("admin-x")),
    `X's late save must not trigger a probe; probes after X resolved: ${JSON.stringify(probeOrigins)}`,
  );

  // Resolve Y — its probe fires normally with originY.
  resolveY?.(originY);
  await settle(20);

  assert.ok(
    probeOrigins.some((o) => o.includes("admin-y")),
    `Y's save must trigger a probe with originY; probes: ${JSON.stringify(probeOrigins)}`,
  );

  await unmount();
});

// ── detail-navigation ────────────────────────────────────────────────────────

test("detail-navigation: stale detail result is discarded after navigating away", async () => {
  // Verifies useAsyncLoad's effect-local active flag on detail fetch.
  //
  // Scenario:
  //  1. Panel renders; list resolves immediately with one entry.
  //  2. User clicks the report row → detail fetch A starts (active=true,
  //     waiting on detailDeferredA).
  //  3. origin/pubkey changes → generation bumps → old effect cleanup:
  //     active=false. New effect starts → detail fetch B (detailDeferredB).
  //  4. detailDeferredA resolves with "STALE-DETAIL-CONTENT" → active=false
  //     → result discarded. detailDeferredB stays pending → UI shows loading.
  //
  // Fails if the `active = false` cleanup is removed: fetch A has active=true,
  // so "STALE-DETAIL-CONTENT" commits and appears in the DOM.

  const origin = "https://admin.example.com";
  const pubkey = "a".repeat(64);

  const listResult = [
    {
      id: "00000000-0000-0000-0000-000000000099",
      communityId: "00000000-0000-0000-0000-000000000002",
      communityHost: "relay.example.com",
      reportEventId: "aa",
      reporterPubkey: "bb",
      targetKind: "event",
      target: "cc",
      reportType: "spam",
      status: "open",
      createdAt: "2024-01-01T00:00:00Z",
    },
  ];

  setIpcHandler("admin_list_reports", () => Promise.resolve(listResult));

  // Two separate deferreds: A for the first (stale) fetch, B for the second.
  // This prevents B from accidentally committing A's stale content when the
  // deferred is shared.
  const detailDeferredA = deferred();
  const detailDeferredB = deferred();
  let detailCallCount = 0;
  setIpcHandler("admin_get_report", () => {
    detailCallCount += 1;
    return detailCallCount === 1
      ? detailDeferredA.promise
      : detailDeferredB.promise;
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });

  // Initial render + list resolution.
  await act(async () => {
    await doRender();
    await new Promise((r) => setTimeout(r, 30));
  });

  // Find a report row button and click via fireEvent.
  const allButtons = container.querySelectorAll("button");
  let clickedReport = false;
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 0));
    });
    clickedReport = true;
    break;
  }

  assert.ok(clickedReport, "a report row button must exist and be clickable");

  // Detail fetch A is in-flight (active=true). Change origin/pubkey →
  // generation bumps → old effect cleanup: active=false. New effect starts
  // (active=true) and calls admin_get_report → detailDeferredB.
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  await act(async () => {
    await doRender({
      origin: "https://admin-2.example.com",
      pubkey: "b".repeat(64),
    });
    await new Promise((r) => setTimeout(r, 5));
  });

  // Resolve stale fetch A. Its active=false → result discarded.
  detailDeferredA.resolve({
    id: "00000000-0000-0000-0000-000000000099",
    content: "STALE-DETAIL-CONTENT",
    status: "STALE-DETAIL",
  });

  await act(async () => {
    await new Promise((r) => setTimeout(r, 30));
  });

  const text = container.textContent ?? "";
  assert.ok(
    !text.includes("STALE-DETAIL-CONTENT"),
    `stale detail A must not appear (active=false); got: ${text.slice(0, 300)}`,
  );

  // Clean up: resolve B to avoid dangling promises.
  detailDeferredB.resolve({ id: "skip", content: "done" });
  await act(async () => {
    await new Promise((r) => setTimeout(r, 5));
  });

  await unmount();
});

// ── blob-leak-on-back-navigation ──────────────────────────────────────────────────────────────

test("blob-leak-on-back-navigation: loadGenRef cleanup prevents orphaned blob URL", async () => {
  // Isolates the loadGenRef.current += 1 cleanup in AttachmentViewer.
  //
  // Scenario: attachment fetch is in-flight, then the user navigates "Back to
  // feedback" (onBack sets selectedId=null in FeedbackTab, unmounting
  // FeedbackDetail and AttachmentViewer). At unmount the cleanup fires:
  //   loadGenRef.current += 1  ← MUTATION TARGET
  // The late fetch resolves. Since origin/pubkey are UNCHANGED (no context
  // change happened), only the loadGenRef check catches the mismatch:
  //   thisGen (pre-cleanup value) !== loadGenRef.current (incremented) → revoke
  //
  // Without the cleanup increment:
  //   thisGen === loadGenRef.current (both remain at 1) → all three guards pass
  //   → setBlobUrl called → blob URL committed to blobUrlRef.current with no
  //   revocation → orphaned blob URL leak.
  //
  // Fails if loadGenRef.current += 1 is removed from the cleanup.

  const origin = "https://admin.example.com";
  const pubkey = "a".repeat(64);
  const sha256 = "a".repeat(64);

  const feedbackSummary = {
    id: "00000000-0000-0000-0000-000000000011",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    submitterPubkey: "submitterblobtest001",
    category: null,
    bodySummary: "Test feedback summary",
    receivedAt: "2024-01-01T00:00:01Z",
  };

  const feedbackDetail = {
    id: "00000000-0000-0000-0000-000000000011",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    eventId: "blobtest001",
    submitterPubkey: "submitterblobtest001",
    category: null,
    body: "Test feedback full body",
    tags: [
      [
        "imeta",
        `url https://relay.example.com/files/${sha256}`,
        "m image/png",
        `x ${sha256}`,
        "size 1000",
      ],
    ],
    eventCreatedAt: "2024-01-01T00:00:00Z",
    receivedAt: "2024-01-01T00:00:01Z",
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () =>
    Promise.resolve([feedbackSummary]),
  );
  setIpcHandler("admin_get_feedback", () => Promise.resolve(feedbackDetail));

  const attachDeferred = deferred();
  const revokedUrls = [];
  const origRevoke = globalThis.URL?.revokeObjectURL;
  if (!globalThis.URL) globalThis.URL = {};
  globalThis.URL.revokeObjectURL = (url) => {
    revokedUrls.push(url);
    if (origRevoke) origRevoke.call(globalThis.URL, url);
  };
  globalThis.URL.createObjectURL = () => "blob:back-nav-test-url";
  setIpcHandler(
    "admin_fetch_feedback_attachment",
    () => attachDeferred.promise,
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });

  await act(async () => {
    await doRender();
    await new Promise((r) => setTimeout(r, 30));
  });

  // Click Feedback tab.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });

  // Navigate to feedback detail. Image attachments auto-load on mount,
  // so navigating to the detail starts the load immediately — no "View
  // attachment" click needed.
  let navigatedToDetail = false;
  for (const btn of container.querySelectorAll("button")) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    navigatedToDetail = true;
    break;
  }
  assert.ok(
    navigatedToDetail,
    "must navigate to feedback detail and start attachment load",
  );

  // Attachment fetch is now in-flight.  Click "Back to feedback" — this
  // unmounts FeedbackDetail (and AttachmentViewer within it) WITHOUT changing
  // origin or pubkey.  The cleanup fires: loadGenRef.current += 1.
  const backBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    (b.textContent ?? "").includes("Back to feedback"),
  );
  assert.ok(
    backBtn,
    "'Back to feedback' button must be present while detail is showing",
  );
  await act(async () => {
    fireEvent.click(backBtn);
    await new Promise((r) => setTimeout(r, 5));
  });

  // Resolve the attachment fetch.  With cleanup increment:
  //   thisGen (1) !== loadGenRef.current (2) → URL.revokeObjectURL("blob:back-nav-test-url")
  // Without cleanup increment:
  //   thisGen (1) === loadGenRef.current (1) AND origin/pubkey unchanged
  //   → setBlobUrl called → orphaned blob, no revocation.
  attachDeferred.resolve(new ArrayBuffer(8));
  await act(async () => {
    await new Promise((r) => setTimeout(r, 30));
  });

  assert.ok(
    revokedUrls.includes("blob:back-nav-test-url"),
    `blob URL must be revoked on back-navigation; revokedUrls: ${JSON.stringify(revokedUrls)}`,
  );

  if (origRevoke !== undefined) globalThis.URL.revokeObjectURL = origRevoke;
  await unmount();
});

// ── cross-identity delayed save ───────────────────────────────────────────────

test("cross-identity-delayed-save: A's late save carries A's expectedPubkey and does not touch B's state", async () => {
  // Verifies that set_admin_origin IPC is called with expectedPubkey = A's pubkey,
  // and that A's late save completion does not alter B's component state.
  //
  // The cross-session boundary is enforced by key={pubkeyHex}: when pubkey changes,
  // A's component unmounts and B's mounts fresh. A's deferred save resolves and
  // its continuation calls runProbe — but React state updates on the unmounted A
  // component are discarded. B's input and panel are unaffected.
  //
  // Scenario:
  //  1. Mount with pubkeyA; drive to authorized (probe nip98Authorized, panel rendered).
  //  2. Edit input and start save — deferred set_admin_origin with expectedPubkey=A.
  //  3. Switch identity to pubkeyB while A's save is pending:
  //     - A's component is synchronously unmounted (key change).
  //     - B's component mounts fresh with no saved origin.
  //  4. Resolve A's deferred save late.
  //  5. Assert:
  //     a. The set_admin_origin call recorded expectedPubkey = pubkeyA.
  //     b. B's input is still empty (A's late state writes discarded by React).
  //     c. B's panel does not show A's origin as authorized.
  //     d. No admin_probe fires for A's origin after the identity switch.
  //
  // Fails if expectedPubkey is dropped from the set_admin_origin invocation path
  // (api.ts forwarding): the recorded call has no expectedPubkey, so the Rust-level
  // guard cannot enforce identity isolation.

  const pubkeyA = "a".repeat(64);
  const pubkeyB = "b".repeat(64);
  const originA = "https://admin-a.example.com";
  const newOriginA = "https://admin-a-new.example.com";

  // Saved origin for A; B has none.
  setIpcHandler("get_admin_origin", (args) => {
    if (args?.expectedPubkey === pubkeyA) return Promise.resolve(originA);
    return Promise.resolve(null);
  });
  // Initial probe for A → authorized so the panel renders.
  setIpcHandler("admin_probe", () =>
    Promise.resolve({ state: "nip98Authorized" }),
  );
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const qc = makeQueryClient(pubkeyA);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(30);

  // A is authorized — input must show originA.
  const inputA = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(inputA, "input must render for pubkeyA");

  // Record all set_admin_origin calls.
  const saveRecords = [];
  let resolveSaveA;
  setIpcHandler("set_admin_origin", (args) => {
    saveRecords.push({ ...args });
    return new Promise((r) => {
      resolveSaveA = r;
    });
  });

  // Edit input to newOriginA and press Enter to start a deferred save.
  await act(async () => {
    fireEvent.change(inputA, { target: { value: newOriginA } });
    await new Promise((r) => setTimeout(r, 5));
  });
  await act(async () => {
    fireEvent.keyDown(inputA, { key: "Enter", keyCode: 13 });
    await new Promise((r) => setTimeout(r, 5));
  });

  // A's save is now in-flight (deferred). Switch to pubkeyB.
  // A's component is synchronously unmounted (key change).
  setIpcHandler("get_admin_origin", (args) => {
    if (args?.expectedPubkey === pubkeyB) return Promise.resolve(null);
    return Promise.resolve(null);
  });
  // After switch, record admin_probe calls to detect any stale A probe firing.
  const probeRecords = [];
  setIpcHandler("admin_probe", (args) => {
    probeRecords.push({ ...args });
    return Promise.resolve({ state: "disabled" });
  });
  await act(async () => {
    qc.setQueryData(["identity"], { pubkey: pubkeyB });
    await new Promise((r) => setTimeout(r, 20));
  });

  // Resolve A's deferred save late. A's component is already unmounted — any
  // React state updates from A's continuation are discarded. B remains untouched.
  resolveSaveA?.(newOriginA);
  await settle(30);

  // (a) The set_admin_origin IPC call must have carried expectedPubkey = pubkeyA.
  assert.ok(
    saveRecords.length >= 1,
    "set_admin_origin must have been called at least once",
  );
  assert.equal(
    saveRecords[0]?.expectedPubkey,
    pubkeyA,
    `set_admin_origin must carry expectedPubkey = pubkeyA; got: ${JSON.stringify(saveRecords[0])}`,
  );

  // (b) B's input must still be empty (A's late state writes are discarded by React
  // on the unmounted A component; they never reach B's component tree).
  const inputB = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(inputB, "B's input must be present after identity switch");
  assert.equal(
    inputB.value,
    "",
    `B's input must be empty after identity switch; got: "${inputB.value}"`,
  );

  // (c) B's panel must not show A's origin as authorized — B is not authorized.
  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.equal(
    panel,
    null,
    "admin-console-panel must not render for B — B has no authorized origin",
  );

  // (d) No admin_probe must have fired for A's origin after the identity switch.
  // A's handleSave continuation calls runProbe(canonical) after the save resolves.
  // The sessionTokenRef check prevents same-session concurrent saves from firing
  // a stale probe, but it does not stop A's own continuation after A unmounts:
  // A's sessionTokenRef still matches A's token, so the check passes and
  // runProbe(newOriginA) fires as an IPC call. React discards the state update
  // on the unmounted component, so B is unaffected — but the probe IPC fires.
  // This assertion catches any such stale probe call: if a probe with A's origin
  // is recorded here, production code is calling probeAdminOrigin after unmount.
  const staleProbe = probeRecords.find(
    (p) => p?.origin === originA || p?.origin === newOriginA,
  );
  assert.equal(
    staleProbe,
    undefined,
    `no admin_probe must fire for A's origin after identity switch; got: ${JSON.stringify(staleProbe)}`,
  );

  await unmount();
});

// ── strict-mode-save ──────────────────────────────────────────────────────────

test("strict-mode-save: probe fires after save under React.StrictMode double-mount", async () => {
  // Verifies the StrictMode-safe unmount fence in AdminConsoleSettingsSession.
  //
  // React.StrictMode (used in desktop/src/main.tsx) double-invokes effects in
  // development:  setup → cleanup → setup.  An isMountedRef-based fence
  // (cleanup sets isMountedRef.current = false, no reset in setup body) leaves
  // the ref permanently false after the double-mount, silently killing every
  // save completion in dev builds.
  //
  // The correct fence nulls sessionTokenRef on unmount instead:
  //   useEffect(() => () => { sessionTokenRef.current = null; }, [])
  // StrictMode's cleanup sets sessionTokenRef.current = null, then the setup
  // re-runs handleSave's `sessionTokenRef.current = token` when a new save
  // starts — so the fence is re-armed per save, not per mount.
  //
  // Fails if the unmount-cleanup effect is removed (isMountedRef variant or no
  // fence): after StrictMode double-mount, handleSave continuation is
  // permanently blocked (isMountedRef=false), so probeOrigins stays empty.

  const pubkey = "c".repeat(64);
  const savedOrigin = "https://admin-strict.example.com";
  const canonicalOrigin = "https://admin-strict-canonical.example.com";

  setIpcHandler("get_admin_origin", () => Promise.resolve(savedOrigin));

  // Track probe invocations to verify the save drives a probe.
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({ state: "disabled" });
  });
  setIpcHandler("set_admin_origin", () => Promise.resolve(canonicalOrigin));
  setIpcHandler("get_users_batch", () =>
    Promise.resolve({ profiles: {}, missing: [] }),
  );

  // gcTime: Infinity is critical: with gcTime: 0 StrictMode's simulated unmount
  // GCs the seeded identity query before the component's observer re-subscribes,
  // so the input never renders on the second mount.
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  qc.setQueryData(["identity"], { pubkey });

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  // Mount under React.StrictMode — triggers setup → cleanup → setup on all effects.
  await act(async () => {
    root.render(
      React.createElement(
        React.StrictMode,
        null,
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(AdminConsoleSettingsCard),
          ),
        ),
      ),
    );
  });
  await settle(30);

  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(
    input,
    "origin input must render after StrictMode double-mount — identity query not GC'd",
  );

  // Clear probes from the initial mount probe.
  probeOrigins.length = 0;

  // Edit input and press Enter to trigger handleSave().
  const newOrigin = "https://admin-strict-new.example.com";
  await act(async () => {
    fireEvent.change(input, { target: { value: newOrigin } });
    await new Promise((r) => setTimeout(r, 5));
  });
  await act(async () => {
    fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
    await new Promise((r) => setTimeout(r, 5));
  });
  await settle(30);

  // The probe must fire for the canonical origin returned by set_admin_origin.
  // Fails if isMountedRef=false (from StrictMode cleanup) permanently blocks
  // the handleSave continuation: probeOrigins stays empty.
  assert.ok(
    probeOrigins.some((o) => o === canonicalOrigin),
    `probe must fire after save under StrictMode; probes: ${JSON.stringify(probeOrigins)}`,
  );

  await act(async () => {
    root.unmount();
  });
  document.body.removeChild(container);
});

// ── NIP-11 auto-discovery ─────────────────────────────────────────────────

test("discovery-success: a same-host discovered origin is auto-saved and auto-probed — panel renders without Save", async () => {
  // Verifies item 1 (render without Save): when get_admin_origin returns null,
  // the card discovers the relay's admin_api and — because it is same-host
  // (sameHost === true, the advertised host matches the connected relay) —
  // auto-saves it via set_admin_origin (same validation path as an explicit
  // Save), then probes it. The panel renders immediately without the operator
  // clicking Save. The cross-host gate is covered by discovery-cross-host.
  //
  // The relay we are already connected to is a trusted source; the Rust
  // AdminOrigin::parse gate validates the discovered value before storing or
  // signing against it. If validation fails, the code falls back to pre-fill
  // only (tested in discovery-save-fails-falls-back test below).
  //
  // Fails if the mount effect reverts to pre-fill-only behavior:
  // admin_probe would not fire and the panel would not render without Save.

  const pubkey = "1".repeat(64);
  const discovered = "http://127.0.0.1:3000";
  const canonical = discovered;

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  let discoverCalls = 0;
  setIpcHandler("admin_discover_origin", () => {
    discoverCalls += 1;
    return Promise.resolve({ origin: discovered, sameHost: true });
  });
  let saveCalls = 0;
  setIpcHandler("set_admin_origin", (args) => {
    saveCalls += 1;
    return Promise.resolve(args?.rawOrigin ?? canonical);
  });
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({
      state: "nip98Authorized",
      role: "operator",
      source: "config",
    });
  });
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(50);

  assert.equal(
    discoverCalls,
    1,
    "admin_discover_origin must be called once when no origin is saved",
  );
  assert.equal(
    saveCalls,
    1,
    "set_admin_origin must be called to persist the discovered origin",
  );
  assert.deepEqual(
    probeOrigins,
    [canonical],
    `the discovered origin must be probed automatically; got: ${JSON.stringify(probeOrigins)}`,
  );

  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    canonical,
    `input must show the auto-saved origin; got: "${input?.value}"`,
  );

  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.ok(
    panel !== null,
    "admin-console-panel must render after auto-probe without requiring a Save click",
  );

  await unmount();
});

test("discovery-cross-host: a cross-host advertisement is pre-filled only — no auto-save, no auto-probe (unconsented-signature gate)", async () => {
  // Security gate (F1): a relay may advertise an admin_api on a host it does
  // not own. Auto-probing signs a NIP-98 header with the operator's key, so a
  // cross-host advertisement (sameHost === false) must NOT be saved or probed
  // automatically — it is pre-filled under Advanced for explicit operator
  // review. Same-host advertisements keep the auto-save + auto-probe UX
  // (covered by discovery-success).
  //
  // Falsifiable: if the sameHost gate is removed, the effect would auto-save
  // and auto-probe the cross-host origin exactly like discovery-success — so
  // set_admin_origin and admin_probe would fire. Both are asserted absent here,
  // and the pre-filled input + open Advanced disclosure are asserted present.

  const pubkey = "6".repeat(64);
  const discovered = "https://evil.attacker.example.com";

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  let discoverCalls = 0;
  setIpcHandler("admin_discover_origin", () => {
    discoverCalls += 1;
    return Promise.resolve({ origin: discovered, sameHost: false });
  });
  let saveCalls = 0;
  setIpcHandler("set_admin_origin", (args) => {
    saveCalls += 1;
    return Promise.resolve(args?.rawOrigin ?? discovered);
  });
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({ state: "nip98Authorized", role: "operator" });
  });

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(50);

  assert.equal(
    discoverCalls,
    1,
    "admin_discover_origin must be called once when no origin is saved",
  );
  assert.equal(
    saveCalls,
    0,
    "set_admin_origin must NOT be called for a cross-host advertisement — the operator saves explicitly",
  );
  assert.deepEqual(
    probeOrigins,
    [],
    `no probe (and no NIP-98 signature) must fire for a cross-host advertisement; got: ${JSON.stringify(probeOrigins)}`,
  );

  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    discovered,
    `the cross-host origin must be pre-filled for manual review; got: "${input?.value}"`,
  );
  const disclosure = container.querySelector("details.group\\/advanced");
  assert.ok(
    disclosure?.open,
    "the Advanced disclosure must be open so the operator can see the pre-filled value awaiting Save",
  );
  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.equal(
    panel,
    null,
    "admin-console-panel must NOT render for an unsaved, unprobed cross-host origin",
  );

  await unmount();
});

test("discovery-save-fails-falls-back: if set_admin_origin rejects for discovered origin, falls back to pre-fill only", async () => {
  // When AdminOrigin::parse rejects the discovered value (e.g. invalid URL),
  // set_admin_origin throws. The code must fall back to pre-fill + Advanced
  // open (the old behavior) rather than surfacing an error or probing.
  //
  // Fails if the save-failure path is removed: an invalid discovered origin
  // would cause an error state instead of a clean manual-entry fallback.

  const pubkey = "5".repeat(64);
  const discovered = "not-a-valid-origin";

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  setIpcHandler("admin_discover_origin", () =>
    Promise.resolve({ origin: discovered, sameHost: true }),
  );
  setIpcHandler("set_admin_origin", () =>
    Promise.reject(new Error("invalid origin format")),
  );
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({ state: "disabled" });
  });

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(30);

  assert.deepEqual(
    probeOrigins,
    [],
    `no probe must fire when discovery save fails; got: ${JSON.stringify(probeOrigins)}`,
  );
  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    discovered,
    `input must be pre-filled with the discovered origin as fallback; got: "${input?.value}"`,
  );
  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.equal(
    panel,
    null,
    "admin-console-panel must NOT render when discovery save failed",
  );

  await unmount();
});

// ── Discovery fallback — null/rejection table ──────────────────────────────
//
// Both paths share the same observable outcome: empty input, no probe, no
// panel. They differ in how admin_discover_origin behaves and in one extra
// assertion that the rejection path never surfaces an error badge.
//
// Keep each row's own discovery mock so the distinction is visible.

const DISCOVERY_FALLBACK_ROWS = [
  {
    name: "absent",
    description:
      "no saved origin and no advertised admin_api falls back to manual entry",
    pubkey: "2".repeat(64),
    // Fails if discovery null is not treated as "fall back": a probe would fire
    // for a null/empty origin or the panel would render.
    setupDiscovery: (trackCalls) => {
      setIpcHandler("admin_discover_origin", () => {
        trackCalls.count += 1;
        return Promise.resolve(null);
      });
    },
    extraAssert: null,
  },
  {
    name: "error",
    description:
      "a failed discovery fetch falls back to manual entry without surfacing an error",
    pubkey: "3".repeat(64),
    // The relay-side admin_api validation lives in Rust: an advertised-but-invalid
    // value resolves to null there. A transport error rejects the promise; the
    // card swallows it and falls back to manual entry rather than showing an
    // error badge (discovery is best-effort, not operator action).
    //
    // Fails if the discovery try/catch is removed: the rejection propagates to
    // the outer catch and the card renders an error badge instead of a clean
    // manual-entry state.
    setupDiscovery: (_trackCalls) => {
      setIpcHandler("admin_discover_origin", () =>
        Promise.reject(new Error("relay unreachable: network error")),
      );
    },
    extraAssert: (container) => {
      const text = container.textContent ?? "";
      assert.ok(
        !text.includes("network error"),
        `a best-effort discovery error must not surface as an error badge; got: ${text.slice(0, 200)}`,
      );
    },
  },
];

for (const row of DISCOVERY_FALLBACK_ROWS) {
  test(`discovery-${row.name}: ${row.description}`, async () => {
    const discoverTracker = { count: 0 };
    setIpcHandler("get_admin_origin", () => Promise.resolve(null));
    row.setupDiscovery(discoverTracker);
    const probeOrigins = [];
    setIpcHandler("admin_probe", (args) => {
      probeOrigins.push(args?.origin ?? "(none)");
      return Promise.resolve({ state: "disabled" });
    });

    const qc = makeQueryClient(row.pubkey);
    const { container, doRender, unmount } = mountCard(qc);
    await doRender();
    await settle(30);

    if (row.name === "absent") {
      assert.equal(
        discoverTracker.count,
        1,
        "admin_discover_origin must be attempted",
      );
    }
    assert.deepEqual(
      probeOrigins,
      [],
      `no probe must fire when discovery ${row.name === "absent" ? "returns null" : "errors"}; got: ${JSON.stringify(probeOrigins)}`,
    );

    const input = container.querySelector("[data-testid='admin-origin-input']");
    assert.equal(
      input?.value,
      "",
      `input must be empty for manual entry when discovery ${row.name === "absent" ? "finds nothing" : "errors"}; got: "${input?.value}"`,
    );
    const panel = container.querySelector(
      "[data-testid='admin-console-panel']",
    );
    assert.equal(
      panel,
      null,
      `admin-console-panel must not render when there is no discovered origin (${row.name})`,
    );

    if (row.extraAssert) row.extraAssert(container);

    await unmount();
  });
}

test("discovery-skipped: a saved origin takes precedence and discovery is not attempted", async () => {
  // Verifies the manual-fallback-wins invariant: an explicitly saved origin
  // is probed directly and admin_discover_origin is never called.
  //
  // Fails if discovery runs unconditionally and clobbers the saved origin.

  const pubkey = "4".repeat(64);
  const saved = "https://admin.example.com";

  setIpcHandler("get_admin_origin", () => Promise.resolve(saved));
  let discoverCalls = 0;
  setIpcHandler("admin_discover_origin", () => {
    discoverCalls += 1;
    return Promise.resolve({ origin: "http://127.0.0.1:3000", sameHost: true });
  });
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({ state: "disabled" });
  });

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(30);

  assert.equal(
    discoverCalls,
    0,
    "admin_discover_origin must NOT be called when an origin is already saved",
  );
  assert.deepEqual(
    probeOrigins,
    [saved],
    `the saved origin must be probed, not a discovered one; got: ${JSON.stringify(probeOrigins)}`,
  );
  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    saved,
    `input must show the saved origin; got: "${input?.value}"`,
  );

  await unmount();
});

// ── P2-1 Settings→panel wiring: onSelfMutation propagates from SettingsCard ──

test("settings-card-self-demotion-reruns-probe: self-demotion through SettingsCard triggers runProbe", async () => {
  // Verifies the Settings→panel wiring at AdminConsoleSettingsCard.tsx:462:
  //   onSelfMutation={() => runProbe(savedOrigin)}
  //
  // The existing staffing-self-demotion-fires-onSelfMutation test mounts
  // AdminConsolePanel directly with onSelfMutation as a prop — it proves the
  // StaffingTab guard fires but says nothing about whether SettingsCard passes
  // the callback. This test mounts the real AdminConsoleSettingsCard and
  // confirms the full path: SettingsCard→panel wiring → Staffing mutation →
  // onSelfMutation → runProbe → probe IPC called a second time → new role
  // reflected in UI → Staffing tab disappears.
  //
  // Mutation evidence: remove the `onSelfMutation={() => runProbe(savedOrigin)}`
  // prop at SettingsCard.tsx:462 → AdminConsolePanel receives no callback →
  // StaffingTab's onSelfMutation?.() fires nothing → second probe never called →
  // probeCallCount stays at 1 → Staffing tab remains visible → test RED.

  const pubkey = "cc".repeat(32); // self
  const otherPubkey = "dd".repeat(32); // another operator
  const savedOrigin = "https://admin-settings-self-demote.example.com";

  let probeCallCount = 0;
  // First probe: self is operator. Second probe (after self-demotion): moderator.
  setIpcHandler("admin_probe", () => {
    probeCallCount += 1;
    if (probeCallCount === 1) {
      return Promise.resolve({
        state: "nip98Authorized",
        role: "operator",
        source: "db",
      });
    }
    return Promise.resolve({
      state: "nip98Authorized",
      role: "moderator",
      source: "db",
    });
  });
  setIpcHandler("get_admin_origin", () => Promise.resolve(savedOrigin));
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve([
      { pubkey: pubkey, effectiveRole: "operator", sources: ["db"] },
      { pubkey: otherPubkey, effectiveRole: "operator", sources: ["db"] },
    ]),
  );
  setIpcHandler("admin_put_operator", () =>
    Promise.resolve({
      pubkey: pubkey,
      effectiveRole: "moderator",
      sources: ["db"],
    }),
  );
  setIpcHandler("get_users_batch", () =>
    Promise.resolve({ profiles: {}, missing: [] }),
  );

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(60);

  // After initial probe: operator role → Staffing tab must be visible.
  const staffingTabBefore = container.querySelector(
    "[data-testid='admin-tab-staffing']",
  );
  assert.ok(
    staffingTabBefore !== null,
    "Staffing tab must render initially when probe returns operator role",
  );
  assert.equal(probeCallCount, 1, "probe must have been called once on mount");

  // Navigate to the Staffing tab.
  await act(async () => {
    fireEvent.click(staffingTabBefore);
    await new Promise((r) => setTimeout(r, 30));
  });

  // Self role selector must now be present.
  const selfRoleSelect = container.querySelector(
    `[data-testid='staffing-role-select-${pubkey}']`,
  );
  assert.ok(
    selfRoleSelect !== null,
    "self role selector must be present after navigating to Staffing tab",
  );

  // Demote self: change own role from operator → moderator.
  await act(async () => {
    fireEvent.change(selfRoleSelect, { target: { value: "moderator" } });
    await new Promise((r) => setTimeout(r, 60));
  });

  // The SettingsCard wiring must have called runProbe a second time.
  assert.equal(
    probeCallCount,
    2,
    `admin_probe must be called a second time after self-demotion via SettingsCard wiring; ` +
      `called ${probeCallCount} times. Remove onSelfMutation={() => runProbe(savedOrigin)} at ` +
      "SettingsCard.tsx:462 to reproduce this failure.",
  );

  // After the second probe returns moderator: Staffing tab must be gone.
  const staffingTabAfter = container.querySelector(
    "[data-testid='admin-tab-staffing']",
  );
  assert.equal(
    staffingTabAfter,
    null,
    "Staffing tab must disappear after self-demotion triggers re-probe returning moderator role",
  );

  // Role badge must now reflect moderator.
  const text = container.textContent ?? "";
  assert.ok(
    text.includes("moderator"),
    `role badge must show "moderator" after self-demotion re-probe; got: ${text.slice(0, 300)}`,
  );

  await unmount();
});

test("settings-card-other-demotion-does-not-reruns-probe: demoting a different operator does NOT re-run probe", async () => {
  // Negative control for the wiring test above.
  // Mutating a different operator's role must NOT trigger runProbe via
  // onSelfMutation — only self-mutations trigger that callback.
  //
  // Mutation evidence: change the `op.pubkey === pubkey` guard in StaffingTab
  // to always call onSelfMutation?.() → probeCallCount becomes 2 after the
  // other-operator mutation → test RED.

  const pubkey = "ee".repeat(32); // self
  const otherPubkey = "ff".repeat(32); // different operator being demoted
  const savedOrigin = "https://admin-settings-other-demote.example.com";

  let probeCallCount = 0;
  setIpcHandler("admin_probe", () => {
    probeCallCount += 1;
    return Promise.resolve({
      state: "nip98Authorized",
      role: "operator",
      source: "db",
    });
  });
  setIpcHandler("get_admin_origin", () => Promise.resolve(savedOrigin));
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve([
      { pubkey: pubkey, effectiveRole: "operator", sources: ["db"] },
      { pubkey: otherPubkey, effectiveRole: "operator", sources: ["db"] },
    ]),
  );
  setIpcHandler("admin_put_operator", () =>
    Promise.resolve({
      pubkey: otherPubkey,
      effectiveRole: "moderator",
      sources: ["db"],
    }),
  );
  setIpcHandler("get_users_batch", () =>
    Promise.resolve({ profiles: {}, missing: [] }),
  );

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(60);

  assert.equal(probeCallCount, 1, "probe must be called once on mount");

  // Navigate to the Staffing tab.
  const staffingTab = container.querySelector(
    "[data-testid='admin-tab-staffing']",
  );
  assert.ok(staffingTab !== null, "Staffing tab must be visible for operator");
  await act(async () => {
    fireEvent.click(staffingTab);
    await new Promise((r) => setTimeout(r, 30));
  });

  // Other operator's role selector must be present.
  const otherRoleSelect = container.querySelector(
    `[data-testid='staffing-role-select-${otherPubkey}']`,
  );
  assert.ok(
    otherRoleSelect !== null,
    "other operator's role selector must be present in Staffing tab",
  );

  // Demote the OTHER operator.
  await act(async () => {
    fireEvent.change(otherRoleSelect, { target: { value: "moderator" } });
    await new Promise((r) => setTimeout(r, 60));
  });

  // probe must NOT have been called again — other-operator mutation is not a self-mutation.
  assert.equal(
    probeCallCount,
    1,
    `admin_probe must NOT be called again after demoting a different operator; called ${probeCallCount} times`,
  );

  // Staffing tab must remain visible (self is still operator).
  const staffingTabAfter = container.querySelector(
    "[data-testid='admin-tab-staffing']",
  );
  assert.ok(
    staffingTabAfter !== null,
    "Staffing tab must remain visible after demoting a different operator (self is still operator)",
  );

  await unmount();
});

test("settings-card-stale-self-mutation-ignored-after-origin-switch: stale self-mutation callback does not override a newer origin's authorized state", async () => {
  // Regression for the deferred-mutation cross-origin race (Carl review
  // PRR_kwDORgXb2s8AAAABOhppRA): a self-mutation callback captured for
  // origin A must be ignored if savedOrigin has advanced to B by the time
  // the callback fires — otherwise runProbe(A) supersedes B's authorized state.
  //
  // Mutation evidence: remove the `if (savedOriginRef.current === originAtRender)`
  // guard in SettingsCard.tsx onSelfMutation → stale runProbe(A) fires →
  // probeCount exceeds 2 → panel shows denied state → test RED.

  const pubkey = "a0".repeat(32); // self
  const otherPubkey = "b1".repeat(32); // second operator (required so self-remove is allowed)

  const originA = "https://relay-a-admin.example.com";
  const originB = "https://relay-b-admin.example.com";

  // Manual-resolve for A's delete so we can let it resolve after Save B.
  let resolveDeleteA = null;
  const deleteAInFlight = new Promise((resolve) => {
    resolveDeleteA = resolve;
  });

  let probeCount = 0;
  const probeOrigins = [];
  // Call 1: A authorized (operator) on mount.
  // Call 2: B authorized (operator) after Save B.
  // Call 3+ would mean the stale fence failed — must NOT happen.
  //
  // The mock discriminates by origin so the "no Access denied" check
  // actually detects a stale fence: if call 3 fires for originA it returns
  // nip98Denied, which would render "Access denied" in the panel — making
  // both the probeCount assertion and the text assertion fail for the same
  // defect. Tracking probeOrigins lets us assert the correct probe targets.
  setIpcHandler("admin_probe", (args) => {
    probeCount += 1;
    probeOrigins.push(args?.origin ?? null);
    // Any call after the expected A-mount + B-save pair for origin A is the
    // stale post-removal probe — return denied to surface the fence failure.
    if (probeCount > 2 && args?.origin === originA) {
      return Promise.resolve({ state: "nip98Denied" });
    }
    return Promise.resolve({
      state: "nip98Authorized",
      role: "operator",
      source: "db",
    });
  });

  setIpcHandler("get_admin_origin", () => Promise.resolve(originA));
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve([
      { pubkey: pubkey, effectiveRole: "operator", sources: ["db"] },
      { pubkey: otherPubkey, effectiveRole: "operator", sources: ["db"] },
    ]),
  );
  // Self-remove on A: blocks until resolveDeleteA() fires.
  setIpcHandler("admin_delete_operator", () => deleteAInFlight);
  // Save B returns canonical B immediately.
  setIpcHandler("set_admin_origin", () => Promise.resolve(originB));
  setIpcHandler("get_users_batch", () =>
    Promise.resolve({ profiles: {}, missing: [] }),
  );

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(120);

  assert.equal(probeCount, 1, "should have probed once on mount for A");
  const staffingTab = container.querySelector(
    "[data-testid='admin-tab-staffing']",
  );
  assert.ok(
    staffingTab !== null,
    "Staffing tab must be visible (operator on A)",
  );

  // Navigate to Staffing and start self-removal (DELETE in flight, unresolved).
  await act(async () => {
    fireEvent.click(staffingTab);
    await new Promise((r) => setTimeout(r, 20));
  });

  const removeButton = container.querySelector(
    `[data-testid='staffing-remove-btn-${pubkey}']`,
  );
  assert.ok(removeButton !== null, "self remove button must be present");
  await act(async () => {
    fireEvent.click(removeButton);
    await new Promise((r) => setTimeout(r, 20));
  });

  // AlertDialog portals to document.body, not container.
  const confirmButton = document.body.querySelector(
    "[data-testid='staffing-remove-confirm']",
  );
  assert.ok(confirmButton !== null, "removal confirm button must be present");
  await act(async () => {
    fireEvent.click(confirmButton);
    await new Promise((r) => setTimeout(r, 20));
  });
  // A's DELETE is now in flight and blocked.

  // Save B: updates savedOrigin → B, triggers probe 2 for B (authorized).
  const saveInput = container.querySelector(
    "[data-testid='admin-origin-input']",
  );
  assert.ok(saveInput !== null, "admin origin input must be present");
  await act(async () => {
    fireEvent.change(saveInput, { target: { value: originB } });
    await new Promise((r) => setTimeout(r, 20));
  });
  const saveButton = container.querySelector(
    "[data-testid='admin-origin-save']",
  );
  assert.ok(saveButton !== null, "Save button must be present");
  await act(async () => {
    fireEvent.click(saveButton);
    await new Promise((r) => setTimeout(r, 80));
  });

  assert.equal(
    probeCount,
    2,
    `probe must have fired twice (A-mount + B-save); got ${probeCount}`,
  );
  assert.equal(
    probeOrigins[0],
    originA,
    `first probe must target originA; got: ${probeOrigins[0]}`,
  );
  assert.equal(
    probeOrigins[1],
    originB,
    `second probe must target originB; got: ${probeOrigins[1]}`,
  );

  // B's authorized panel must be visible BEFORE A's DELETE resolves, confirming
  // the new session is correctly established independently of the deferred mutation.
  const panelBeforeDelete = container.querySelector(
    "[data-testid='admin-console-panel']",
  );
  assert.ok(
    panelBeforeDelete !== null,
    "admin-console-panel must be visible for B before A's DELETE resolves",
  );

  // Let A's DELETE resolve — stale onSelfMutation callback fires.
  await act(async () => {
    resolveDeleteA();
    await new Promise((r) => setTimeout(r, 80));
  });

  // Fence must have blocked the third probe (A's origin ≠ current savedOrigin=B).
  assert.equal(
    probeCount,
    2,
    `stale self-mutation must NOT trigger a third probe; probeCount=${probeCount}. ` +
      "Remove the savedOriginRef fence in onSelfMutation (SettingsCard.tsx) to reproduce.",
  );

  // B's authorized panel must still be visible.
  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.ok(
    panel !== null,
    "admin-console-panel must remain visible; B is still authorized",
  );

  // No denied-state text from the stale A probe.
  const text = container.textContent ?? "";
  assert.ok(
    !text.toLowerCase().includes("access denied"),
    `panel must not show 'access denied' after stale A completion; got: ${text.slice(0, 300)}`,
  );

  await unmount();
});

test("settings-card-stale-self-mutation-ignored-after-session-teardown: deferred self-mutation after session unmount does not fire admin_probe", async () => {
  // Regression for Thufir's session-teardown finding (review pass 1/1 on
  // 6dcc6a105): the origin-switch fence protects against a savedOrigin change
  // while the DELETE is in flight, but not against identity teardown.
  //
  // Counterexample without the fix: identity X starts self-removal on origin A;
  // X's Settings session unmounts (pubkeyHex → ""); X's deferred DELETE resolves.
  // The retained onSelfMutation callback closes over savedOriginRef. Without
  // clearing savedOriginRef on unmount, savedOriginRef.current === A and
  // originAtRender === A → fence passes → runProbe(A) fires, signing a NIP-98
  // request with the *currently active* identity's keys (Y's, or none).
  //
  // Fix: unmount cleanup now also nulls savedOriginRef. When the fence runs,
  // savedOriginRef.current is null and null !== A → early return, no probe.
  //
  // Mutation evidence:
  //   Remove `savedOriginRef.current = null` from the unmount cleanup effect in
  //   AdminConsoleSettingsCard.tsx → savedOriginRef retains A on teardown →
  //   fence passes → probeCount reaches 2 → this test goes RED.
  //
  // StrictMode preservation (source-level ordering):
  //   StrictMode fires mount→cleanup→mount. The simulated cleanup nulls
  //   savedOriginRef, but the second mount's load effect calls setSavedOriginBoth
  //   which re-arms the ref. The separate strict-mode-save test explicitly wraps
  //   its tree in React.StrictMode and verifies a post-save probe. The
  //   settings-card-self-demotion-reruns-probe test is not StrictMode-wrapped;
  //   it verifies same-session self-mutation under the normal mount path.

  const pubkey = "a2".repeat(32); // self
  const otherPubkey = "b3".repeat(32); // second operator (required so self-remove is allowed)
  const origin = "https://relay-teardown-admin.example.com";

  // Manual-resolve for the delete — held until after unmount.
  let resolveDelete = null;
  const deleteInFlight = new Promise((resolve) => {
    resolveDelete = resolve;
  });

  let probeCount = 0;
  const probeOrigins = [];
  // Call 1: authorized on mount.
  // Call 2+ would mean the teardown fence failed — must NOT happen after unmount.
  setIpcHandler("admin_probe", (args) => {
    probeCount += 1;
    probeOrigins.push(args?.origin ?? null);
    // After the expected mount probe, return denied for any stale call so
    // a failure is observable in probeCount. The root is unmounted before
    // DELETE resolves, so this test does not assert an "Access denied" render.
    if (probeCount > 1) {
      return Promise.resolve({ state: "nip98Denied" });
    }
    return Promise.resolve({
      state: "nip98Authorized",
      role: "operator",
      source: "db",
    });
  });

  setIpcHandler("get_admin_origin", () => Promise.resolve(origin));
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve([
      { pubkey, effectiveRole: "operator", sources: ["db"] },
      { pubkey: otherPubkey, effectiveRole: "operator", sources: ["db"] },
    ]),
  );
  // Self-remove: blocks until resolveDelete() fires after unmount.
  setIpcHandler("admin_delete_operator", () => deleteInFlight);
  setIpcHandler("get_users_batch", () =>
    Promise.resolve({ profiles: {}, missing: [] }),
  );

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(120);

  assert.equal(probeCount, 1, "should have probed once on mount");
  assert.equal(
    probeOrigins[0],
    origin,
    `mount probe must target origin; got: ${probeOrigins[0]}`,
  );

  const staffingTab = container.querySelector(
    "[data-testid='admin-tab-staffing']",
  );
  assert.ok(staffingTab !== null, "Staffing tab must be visible (operator)");

  // Navigate to Staffing and start self-removal (DELETE in flight, unresolved).
  await act(async () => {
    fireEvent.click(staffingTab);
    await new Promise((r) => setTimeout(r, 20));
  });

  const removeButton = container.querySelector(
    `[data-testid='staffing-remove-btn-${pubkey}']`,
  );
  assert.ok(removeButton !== null, "self remove button must be present");
  await act(async () => {
    fireEvent.click(removeButton);
    await new Promise((r) => setTimeout(r, 20));
  });

  // AlertDialog portals to document.body.
  const confirmButton = document.body.querySelector(
    "[data-testid='staffing-remove-confirm']",
  );
  assert.ok(confirmButton !== null, "removal confirm button must be present");
  await act(async () => {
    fireEvent.click(confirmButton);
    await new Promise((r) => setTimeout(r, 20));
  });
  // DELETE is now in flight and blocked.

  // Unmount the entire session — simulates identity teardown (pubkeyHex → "").
  // This fires the cleanup effect, nulling both sessionTokenRef and savedOriginRef.
  await unmount();

  // Now let the deferred DELETE resolve. The retained onSelfMutation closure
  // runs and reaches the savedOriginRef fence.
  await act(async () => {
    resolveDelete();
    await new Promise((r) => setTimeout(r, 80));
  });

  // Fence must have blocked any post-teardown probe.
  assert.equal(
    probeCount,
    1,
    `post-teardown self-mutation must NOT trigger any additional admin_probe; probeCount=${probeCount}. ` +
      "Add `savedOriginRef.current = null` to the unmount cleanup in AdminConsoleSettingsCard.tsx to fix.",
  );
});
