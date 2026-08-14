/**
 * Event-driven behavior tests for AdminConsoleSettingsCard /
 * AdminConsoleSettingsSession and AdminConsolePanel.
 *
 * This file runs with jsdom pre-installed (via --import ./test-jsdom-setup.mjs)
 * so React 19's canUseDOM is true and isInputEventSupported is set correctly.
 * fireEvent from @testing-library/react dispatches native events that travel
 * through React 19's container-level event delegation, reaching production
 * handlers.
 *
 * What these tests prove — they fail if:
 *   - `abortAndResetProbe()` is removed from input onChange
 *     → origin-edit goes red (stale probe commits, panel renders)
 *   - `sessionTokenRef` check is removed from handleSave
 *     → same-session-save-race goes red (stale save clobbers B's input)
 *   - `active = false` cleanup is removed from useAsyncLoad
 *     → detail-navigation goes red (stale detail commits)
 *   - `expectedPubkey` dropped from the set_admin_origin invocation path
 *     → cross-identity-delayed-save goes red (A's save lacks expectedPubkey)
 *   - unmount-cleanup effect removed (sessionTokenRef not nulled on unmount)
 *     → strict-mode-save goes red (StrictMode double-mount silently disables saves)
 *
 * What these tests also prove:
 *   - `loadGenRef.current += 1` cleanup removed from AttachmentViewer
 *     → blob-leak-on-back-navigation goes red (stale blob leaks without revocation)
 *     Note: the existing attachment-unmount test exercises the same guard but via
 *     origin/pubkey re-render which also updates originRef/pubkeyRef. The back-
 *     navigation test isolates loadGenRef by unmounting without context change.
 *   - `pubkeyHex ? <AdminConsoleSettingsSession key=…> : null` render gate removed
 *     → authorized-logout-teardown goes red (empty-pubkey session renders, input present)
 *     Note: this test lives in adminConsolePanel.test.mjs (MinimalDocument suite) because
 *     the jsdom React 19 global scheduler leaves pending promises when the gate is absent,
 *     causing the jsdom test runner to report CANCELLED instead of a clean AssertionError.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

// ── Tauri IPC interceptor ────────────────────────────────────────────────────
//
// @tauri-apps/api/core calls `window.__TAURI_INTERNALS__.invoke(...)` where
// `window` is the jsdom window object (set via test-jsdom-setup.mjs), not
// `globalThis`. Both globalThis.__TAURI_INTERNALS__ and window.__TAURI_INTERNALS__
// must be set so all import paths reach the same mock.

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();

function setIpcHandler(cmd, fn) {
  ipcHandlers.set(cmd, fn);
}
function clearIpcHandlers() {
  ipcHandlers.clear();
}

const tauriMock = {
  invoke(cmd, args) {
    const handler = ipcHandlers.get(cmd);
    if (handler) return handler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback(_cb) {
    return Math.random();
  },
};
// Set on both globalThis and the jsdom window object so all access paths work.
globalThis.__TAURI_INTERNALS__ = tauriMock;
if (globalThis.window && globalThis.window !== globalThis) {
  globalThis.window.__TAURI_INTERNALS__ = tauriMock;
}

// ── Production imports ───────────────────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { toast } from "sonner";

import { AdminConsoleSettingsCard } from "./AdminConsoleSettingsCard.tsx";
import { AdminConsolePanel } from "./AdminConsolePanel.tsx";

// ── Success-toast capture ────────────────────────────────────────────────────
//
// sonner's `toast` is a shared singleton object across import paths (verified),
// so replacing `toast.success` here is observed by the production components.
// Captured messages are asserted by the toast tests and cleared in afterEach.

/** @type {string[]} */
const capturedToasts = [];
toast.success = (msg) => {
  capturedToasts.push(String(msg));
  return 0;
};

/** @type {string[]} */
const capturedErrorToasts = [];
toast.error = (msg) => {
  capturedErrorToasts.push(String(msg));
  return 0;
};

// ── Deferred promise helper ──────────────────────────────────────────────────

function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// ── Mount helpers ────────────────────────────────────────────────────────────

function makeQueryClient(pubkeyHex) {
  // gcTime: Infinity prevents React Query from garbage-collecting setQueryData
  // entries before the component mounts its observer. gcTime: 0 races with
  // the GC timer and is appropriate only for test teardown, not setup.
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  // Always set identity data (even for empty pubkey) so React Query never calls
  // queryFn = getIdentity (which would hit the unmocked IPC).
  // Component reads pubkeyHex = identity?.pubkey ?? "" — so { pubkey: "" }
  // produces pubkeyHex = "" which is the correct logged-out representation.
  qc.setQueryData(["identity"], { pubkey: pubkeyHex });
  return qc;
}

function mountCard(qc) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const doRender = async () => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(AdminConsoleSettingsCard),
        ),
      );
    });
  };
  const unmount = async () => {
    await act(async () => {
      root.unmount();
    });
    document.body.removeChild(container);
  };
  return { container, doRender, unmount };
}

function mountPanel({ origin, pubkey }) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const doRender = async ({ origin: o, pubkey: p } = { origin, pubkey }) => {
    await act(async () => {
      root.render(
        React.createElement(AdminConsolePanel, { origin: o, pubkey: p }),
      );
    });
  };
  const unmount = async () => {
    await act(async () => {
      root.unmount();
    });
    document.body.removeChild(container);
  };
  return { container, doRender, unmount };
}

async function settle(ms = 20) {
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

afterEach(() => {
  clearIpcHandlers();
  capturedToasts.length = 0;
  capturedErrorToasts.length = 0;
});

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

// ── attachment-unmount ───────────────────────────────────────────────────────

test("attachment-unmount: late blob URL is revoked and not committed after panel generation changes", async () => {
  // Verifies AttachmentViewer's loadGenRef cleanup and per-load generation guard.
  //
  // Scenario comments updated for auto-load behavior:
  //  1. Panel renders; Feedback tab clicked; list+detail resolve immediately.
  //  2. "View attachment" button appears (non-image mime, no auto-load); user
  //     clicks it — load starts: thisGen = ++loadGenRef.current = 1. Fetch deferred.
  //  3. Re-render with new origin/pubkey bumps panelGeneration →
  //     AttachmentViewer cleanup: loadGenRef.current += 1 = 2. originRef and
  //     pubkeyRef also update to the new values.
  //  4. Attachment resolves: thisGen(1) !== loadGenRef.current(2) (and also
  //     thisOrigin !== originRef.current) — URL.revokeObjectURL called,
  //     setBlobUrl NOT called.
  //
  // Uses application/pdf (non-image) so the attachment doesn't auto-load on
  // mount — the load is triggered by the "View attachment" button click, keeping
  // the scenario identical to the original test design.

  const origin = "https://admin.example.com";
  const pubkey = "a".repeat(64);
  const sha256 = "a".repeat(64);

  const feedbackSummary = {
    id: "00000000-0000-0000-0000-000000000011",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    submitterPubkey: "submitterattach001",
    category: null,
    bodySummary: "Test feedback summary",
    receivedAt: "2024-01-01T00:00:01Z",
  };

  const feedbackDetail = {
    id: "00000000-0000-0000-0000-000000000011",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    eventId: "attachtest001",
    submitterPubkey: "submitterattach001",
    category: null,
    body: "Test feedback full body",
    tags: [
      [
        "imeta",
        `url https://relay.example.com/files/${sha256}`,
        "m application/pdf",
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
  globalThis.URL.createObjectURL = () => "blob:test-url";
  setIpcHandler(
    "admin_fetch_feedback_attachment",
    () => attachDeferred.promise,
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });

  await act(async () => {
    await doRender();
    await new Promise((r) => setTimeout(r, 30));
  });

  // Click the Feedback tab via fireEvent.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab button must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });

  // Navigate to feedback detail, then wait for the auto-load to start.
  // Image attachments now auto-load on AttachmentViewer mount — no "View
  // attachment" click required; the load kicks off as soon as FeedbackDetail
  // renders the AttachmentViewer.
  let startedAttachmentLoad = false;
  const allBtns = container.querySelectorAll("button");
  for (const btn of allBtns) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    // Click feedback item to navigate to detail.
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    // For non-image MIME (application/pdf), a "View attachment" button appears.
    // Click it to start the load.
    for (const b of container.querySelectorAll("button")) {
      if ((b.textContent ?? "").includes("View attachment")) {
        await act(async () => {
          fireEvent.click(b);
          await new Promise((r) => setTimeout(r, 0));
        });
        startedAttachmentLoad = true;
        break;
      }
    }
    break;
  }

  assert.ok(
    startedAttachmentLoad,
    '"View attachment" button must be found and clicked for non-image attachment',
  );

  // Attachment fetch is in-flight (deferred). Change origin/pubkey to bump
  // panelGeneration — triggers AttachmentViewer cleanup: loadGenRef.current += 1.
  // The new panel renders but the user hasn't clicked "View attachment" again,
  // so loadGenRef.current on the now-unmounted instance's ref = original+1.
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  await act(async () => {
    await doRender({
      origin: "https://admin-2.example.com",
      pubkey: "b".repeat(64),
    });
    await new Promise((r) => setTimeout(r, 0));
  });

  // Resolve the attachment fetch. With the cleanup increment:
  //   thisGen(1) !== loadGenRef.current(2) -> revoke, no blob committed.
  // Without the cleanup increment:
  //   thisGen(1) == loadGenRef.current(1) AND thisOrigin(admin.example.com)
  //   !== originRef.current(admin-2.example.com) -> still revoke (origin check).
  // So this test catches the mutation only if the origin/pubkey check is also
  // removed. The loadGenRef test is most meaningful for detecting same-context
  // concurrent loads — see the comment above. We include it here as defense-
  // in-depth: if both loadGenRef AND the origin check were removed, the stale
  // blob would commit.
  attachDeferred.resolve(new ArrayBuffer(8));
  await act(async () => {
    await new Promise((r) => setTimeout(r, 30));
  });

  const img = container.querySelector("img");
  assert.equal(
    img?.getAttribute("src") ?? null,
    null,
    "stale blob URL must not be committed to an img element after panel generation change",
  );

  if (origRevoke !== undefined) globalThis.URL.revokeObjectURL = origRevoke;
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
          React.createElement(AdminConsoleSettingsCard),
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

// ── structured detail layouts ────────────────────────────────────────────────

test("report-detail-renders-structured-fields: ReportDetail shows field layout, not raw JSON", async () => {
  // Verifies item 3: the report detail view renders data-testid='report-detail-fields'
  // and the status value, not a raw JSON <pre>.
  // Lives here (jsdom) because navigating into a detail requires fireEvent.click
  // for React 19's container-level event delegation.
  //
  // Mutation evidence: revert ReportFields → <pre>{JSON.stringify(...)}</pre>
  // → this test goes red ("report-detail-fields element must render").

  const origin = "https://admin.example.com";
  const pubkey = "5".repeat(64);

  const reportItem = {
    id: "00000000-0000-0000-0000-000000000099",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aabb",
    reporterPubkey: "ccdd",
    targetKind: "event",
    target: "eeff",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };

  // Full AdminReportDetailDto: includes note, resolvedBy, and a nested message.
  const reportDetail = {
    ...reportItem,
    channelId: "00000000-0000-0000-0000-000000000003",
    note: "private moderator note",
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: {
      authorPubkey: "aabbccdd",
      content: "offensive message text",
      createdAt: "2024-05-31T10:00:00Z",
      deletedAt: null,
    },
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([reportItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(reportDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Navigate into the report detail — click the first non-tab button.
  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    break;
  }

  await settle(30);

  const fields = container.querySelector(
    "[data-testid='report-detail-fields']",
  );
  assert.ok(
    fields !== null,
    "report-detail-fields element must render — JSON dump not replaced",
  );

  const text = container.textContent ?? "";
  assert.ok(
    text.includes("open"),
    `report status 'open' must appear in structured layout; got: ${text.slice(0, 400)}`,
  );

  // Must NOT be rendering JSON.stringify output (e.g. key-colon pairs).
  assert.ok(
    !text.includes('"status": "open"'),
    `raw JSON must not be rendered in report detail; got: ${text.slice(0, 400)}`,
  );

  // Real DTO fields: note and nested message content must appear.
  assert.ok(
    text.includes("private moderator note"),
    `report note must render; got: ${text.slice(0, 600)}`,
  );
  assert.ok(
    text.includes("offensive message text"),
    `nested message content must render; got: ${text.slice(0, 600)}`,
  );
  assert.ok(
    text.includes("aabbccdd"),
    `nested message authorPubkey must render; got: ${text.slice(0, 600)}`,
  );

  // Fake fields must NOT appear.
  assert.ok(
    !text.includes("reason"),
    `invented 'reason' field must not render; got: ${text.slice(0, 400)}`,
  );
  assert.ok(
    !text.includes("moderationNote"),
    `invented 'moderationNote' field must not render; got: ${text.slice(0, 400)}`,
  );

  await unmount();
});

test("processing-report-navigable-suppresses-resolve-form: a processing report opens into detail, shows enforcement state, and hides the resolve form", async () => {
  // Thufir finding 4: processing rows must stay navigable. The enforcement
  // state (progress/retry/cancel) lives inside the detail view, so disabling
  // the row hides exactly the UI an operator needs while an action is pending.
  // "Not actionable" means suppress the resolve form, not block navigation.
  //
  // Mutation evidence: re-add `disabled={isProcessing}` to the ReportsTab row →
  // the click never opens detail, report-detail-fields never renders → red.
  // Drop the `isOpen` gate on ResolveReportForm → the resolve form renders for
  // a processing report → the resolve-form-absent assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "f5".repeat(32);

  const processingItem = {
    id: "00000000-0000-0000-0000-000000000010",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aabb",
    reporterPubkey: "ccdd",
    targetKind: "event",
    target: "eeff",
    reportType: "spam",
    status: "processing",
    createdAt: "2024-01-01T00:00:00Z",
  };
  const processingDetail = {
    ...processingItem,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000e1",
      requestId: "00000000-0000-0000-0000-0000000000e2",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "enforcing",
      reason: null,
      expiresAt: null,
      errorMessage: null,
      createdAt: "2024-01-01T00:00:00Z",
      updatedAt: "2024-01-01T00:00:01Z",
    },
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([processingItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(processingDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // The processing row must be a navigable (non-disabled) button.
  const rowButtons = Array.from(container.querySelectorAll("button")).filter(
    (btn) => !(btn.getAttribute("data-testid") ?? "").startsWith("admin-tab"),
  );
  const processingRow = rowButtons.find((btn) =>
    btn.textContent?.includes("spam"),
  );
  assert.ok(processingRow, "processing report row must be present");
  assert.ok(
    !processingRow.disabled,
    "processing report row must stay navigable (not disabled)",
  );

  // Navigate into the detail.
  await act(async () => {
    fireEvent.click(processingRow);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // Detail renders (navigation succeeded).
  assert.ok(
    container.querySelector("[data-testid='report-detail-fields']"),
    "report-detail-fields must render after navigating into a processing report",
  );
  // Enforcement state block is shown for a processing report with an action.
  assert.ok(
    container.querySelector("[data-testid='enforcement-state-block']"),
    "enforcement-state-block must render for a processing report",
  );
  // The resolve form must be suppressed for a non-open (processing) report.
  assert.equal(
    container.querySelector("[data-testid='resolve-report-form']"),
    null,
    "resolve form must NOT render for a processing report",
  );

  await unmount();
});

test("feedback-detail-renders-structured-fields: FeedbackDetail shows field layout, not raw JSON", async () => {
  // Verifies item 3: the feedback detail view renders data-testid='feedback-detail-fields'.
  // Lives here (jsdom) because tab switching and item navigation require fireEvent.click.
  //
  // Mutation evidence: revert FeedbackFields → <pre>{JSON.stringify(...)}</pre>
  // → this test goes red ("feedback-detail-fields element must render").

  const origin = "https://admin.example.com";
  const pubkey = "6".repeat(64);

  // Summary shape returned by GET /admin/feedback (FeedbackSummary wire type).
  const feedbackSummary = {
    id: "00000000-0000-0000-0000-000000000011",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    submitterPubkey: "submitter001pubkey",
    category: "bug",
    bodySummary: "App crashes on startup",
    status: "new",
    receivedAt: "2024-05-01T09:00:05Z",
  };

  // Full AdminFeedbackDto shape returned by GET /admin/feedback/:id.
  const feedbackDetail = {
    id: "00000000-0000-0000-0000-000000000011",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    eventId: "feedevent001",
    submitterPubkey: "submitter001pubkey",
    category: "bug",
    body: "App crashes on startup — full detail body text",
    status: "new",
    tags: [],
    eventCreatedAt: "2024-05-01T09:00:00Z",
    receivedAt: "2024-05-01T09:00:05Z",
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () =>
    Promise.resolve([feedbackSummary]),
  );
  setIpcHandler("admin_get_feedback", () => Promise.resolve(feedbackDetail));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Click the Feedback tab.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });

  await settle(30);

  // Pre-navigation: list row shows the summary body text (bodySummary rendered).
  // Mutation seam: render `body` instead of `bodySummary` → red because summary
  // fixture has no `body` field → row title is blank.
  const listText = container.textContent ?? "";
  assert.ok(
    listText.includes("App crashes on startup"),
    `list row must show bodySummary before navigation; got: ${listText.slice(0, 400)}`,
  );

  // Navigate into the feedback detail — click the first non-tab button.
  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    break;
  }

  await settle(30);

  const fields = container.querySelector(
    "[data-testid='feedback-detail-fields']",
  );
  assert.ok(
    fields !== null,
    "feedback-detail-fields element must render — JSON dump not replaced",
  );

  const text = container.textContent ?? "";
  assert.ok(
    !text.includes('"body":'),
    `raw JSON must not be rendered in feedback detail; got: ${text.slice(0, 400)}`,
  );

  // Real DTO fields must render.
  assert.ok(
    text.includes("submitter001pubkey"),
    `submitterPubkey must render; got: ${text.slice(0, 600)}`,
  );
  assert.ok(
    text.includes("bug"),
    `category must render; got: ${text.slice(0, 600)}`,
  );
  assert.ok(
    text.includes("App crashes on startup"),
    `body must render; got: ${text.slice(0, 600)}`,
  );

  // Fake fields must NOT appear.
  assert.ok(
    !text.includes("appVersion"),
    `invented 'appVersion' field must not render; got: ${text.slice(0, 400)}`,
  );
  assert.ok(
    !text.includes("authorPubkey"),
    `invented 'authorPubkey' field must not render; got: ${text.slice(0, 400)}`,
  );

  // Relative timestamp: formatTimestamp output must match "Xm/h/d ago (...)" shape.
  // The fixture receivedAt is far in the past, so it will be "Nd ago (...)".
  assert.ok(
    /\d+[mhd] ago \(/.test(text) || text.includes("just now ("),
    `relative timestamp must render in "Nm/h/d ago (...)" format; got: ${text.slice(0, 600)}`,
  );

  await unmount();
});

// ── contract-dto-nullable-graceful-degradation ────────────────────────────────

test("contract-dto-nullable-graceful-degradation: report detail renders em-dash for absent nullable fields", async () => {
  // Pins graceful degradation when nullable DTO fields are absent.
  // Asserts that fields that are null/absent render as "—" not as empty or crashing.
  //
  // Mutation evidence: remove the null-guard in DetailRow (change `value != null`
  // to `value !== null`) → the em-dash logic breaks for undefined → test goes red.

  const origin = "https://admin.example.com";
  const pubkey = "7".repeat(64);

  const reportItem = {
    id: "00000000-0000-0000-0000-000000000077",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aabb",
    reporterPubkey: "ccdd",
    targetKind: "pubkey",
    target: "eeff",
    reportType: "nudity",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };

  // Detail has no optional fields set and no nested message.
  const reportDetail = {
    ...reportItem,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([reportItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(reportDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Navigate into report detail.
  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    break;
  }
  await settle(30);

  const fields = container.querySelector(
    "[data-testid='report-detail-fields']",
  );
  assert.ok(fields !== null, "report-detail-fields must render");

  const text = container.textContent ?? "";
  // Em-dash appears for null fields (Note, Channel, Resolved by, etc.).
  assert.ok(
    text.includes("—"),
    `em-dash must appear for null nullable fields; got: ${text.slice(0, 600)}`,
  );
  // Nested message block must NOT render when message is null.
  assert.ok(
    !text.includes("Reported message"),
    `nested message block must not render when message is null; got: ${text.slice(0, 600)}`,
  );

  await unmount();
});

// ── contract-dto-mutation-evidence ────────────────────────────────────────────

test("contract-dto-mutation-evidence-resolvedBy: wrong key lookup makes resolvedBy invisible", async () => {
  // Mutation evidence (a): if ReportFields reads data["resolvedBy"] via a wrong
  // key — or if the key in the DTO type is renamed — the resolvedBy value
  // disappears from the rendered output.
  //
  // This test asserts the CORRECT behaviour: resolvedBy IS rendered.
  // To produce the red output, rename `resolvedBy` → `resolvedByX` in ReportFields.

  const origin = "https://admin.example.com";
  const pubkey = "8".repeat(64);

  const reportItem = {
    id: "00000000-0000-0000-0000-000000000088",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "rr01",
    reporterPubkey: "pp01",
    targetKind: "event",
    target: "tt01",
    reportType: "harassment",
    status: "resolved",
    createdAt: "2024-06-01T12:00:00Z",
  };

  const reportDetail = {
    ...reportItem,
    channelId: null,
    note: "case closed",
    resolvedBy: "moderator_pubkey_hex",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([reportItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(reportDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    break;
  }
  await settle(30);

  const text = container.textContent ?? "";

  // The resolvedBy pubkey must appear.
  // Seam: asserting `data.resolvedBy` reaches the rendered DetailRow value.
  // Mutation: rename `resolvedBy` → `resolvedByX` in ReportFields → "moderator_pubkey_hex" absent → red.
  assert.ok(
    text.includes("moderator_pubkey_hex"),
    `resolvedBy value must render via data.resolvedBy; got: ${text.slice(0, 600)}`,
  );

  // The note must also render.
  assert.ok(
    text.includes("case closed"),
    `note value must render via data.note; got: ${text.slice(0, 600)}`,
  );

  await unmount();
});

test("contract-dto-mutation-evidence-nested-message: removing message block hides content", async () => {
  // Mutation evidence (b): removing the nested message block from ReportFields
  // makes the reported message content invisible.
  //
  // This test asserts the CORRECT behaviour: the nested message IS rendered,
  // and the (deleted) indicator appears when deletedAt is non-null.
  // To produce the red output, remove the `{data.message != null && ...}` block.

  const origin = "https://admin.example.com";
  const pubkey = "9".repeat(64);

  const reportItem = {
    id: "00000000-0000-0000-0000-000000000099",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "rr02",
    reporterPubkey: "pp02",
    targetKind: "event",
    target: "tt02",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };

  const reportDetail = {
    ...reportItem,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: {
      authorPubkey: "msg_author_pubkey",
      content: "buy cheap meds at spamsite.example",
      createdAt: "2024-06-01T11:55:00Z",
      // Non-null deletedAt — exercises the deleted indicator branch.
      deletedAt: "2024-06-01T12:10:00Z",
    },
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([reportItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(reportDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    break;
  }
  await settle(30);

  const text = container.textContent ?? "";

  // Seam: asserting the nested message block renders its content field.
  // Mutation: remove `{data.message != null && ...}` → message content absent → red.
  assert.ok(
    text.includes("buy cheap meds at spamsite.example"),
    `nested message content must render; got: ${text.slice(0, 600)}`,
  );
  assert.ok(
    text.includes("msg_author_pubkey"),
    `nested message authorPubkey must render; got: ${text.slice(0, 600)}`,
  );
  assert.ok(
    text.includes("Reported message"),
    `"Reported message" heading must render; got: ${text.slice(0, 600)}`,
  );
  // Seam: asserting the deleted indicator renders when deletedAt is non-null.
  // Mutation: remove the `{data.message.deletedAt != null && ...}` span → "(deleted)" absent → red.
  assert.ok(
    text.includes("(deleted)"),
    `deleted indicator must render when deletedAt is non-null; got: ${text.slice(0, 600)}`,
  );

  await unmount();
});

// ── NIP-11 auto-discovery ─────────────────────────────────────────────────

test("discovery-success: no saved origin auto-discovers and probes without manual input", async () => {
  // Verifies the mount effect: when get_admin_origin returns null, the card
  // calls admin_discover_origin, and on a discovered origin it seeds the input
  // and auto-probes — the operator types nothing.
  //
  // Fails if the discovery branch is removed from the mount effect: no
  // admin_discover_origin call, empty input, panel never renders.

  const pubkey = "1".repeat(64);
  const discovered = "http://127.0.0.1:3000";

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  let discoverCalls = 0;
  setIpcHandler("admin_discover_origin", () => {
    discoverCalls += 1;
    return Promise.resolve(discovered);
  });
  const probeOrigins = [];
  setIpcHandler("admin_probe", (args) => {
    probeOrigins.push(args?.origin ?? "(none)");
    return Promise.resolve({ state: "nip98Authorized" });
  });
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(30);

  assert.equal(
    discoverCalls,
    1,
    "admin_discover_origin must be called once when no origin is saved",
  );
  assert.deepEqual(
    probeOrigins,
    [discovered],
    `discovered origin must be auto-probed; got: ${JSON.stringify(probeOrigins)}`,
  );

  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    discovered,
    `input must be pre-filled with the discovered origin; got: "${input?.value}"`,
  );

  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.ok(
    panel,
    "admin-console-panel must render after auto-discovered origin authorizes",
  );

  await unmount();
});

test("discovery-absent: no saved origin and no advertised admin_api falls back to manual entry", async () => {
  // Verifies the fallback path: get_admin_origin null + admin_discover_origin
  // null → empty input, no probe fires, no panel — the operator can type a URL.
  //
  // Fails if discovery null is not treated as "fall back": a probe would fire
  // for a null/empty origin or the panel would render.

  const pubkey = "2".repeat(64);

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  let discoverCalls = 0;
  setIpcHandler("admin_discover_origin", () => {
    discoverCalls += 1;
    return Promise.resolve(null);
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

  assert.equal(discoverCalls, 1, "admin_discover_origin must be attempted");
  assert.deepEqual(
    probeOrigins,
    [],
    `no probe must fire when discovery returns null; got: ${JSON.stringify(probeOrigins)}`,
  );

  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    "",
    `input must be empty for manual entry when discovery finds nothing; got: "${input?.value}"`,
  );
  const panel = container.querySelector("[data-testid='admin-console-panel']");
  assert.equal(
    panel,
    null,
    "admin-console-panel must not render when there is no discovered origin",
  );

  await unmount();
});

test("discovery-error: a failed discovery fetch falls back to manual entry without surfacing an error", async () => {
  // The relay-side admin_api validation lives in Rust: an advertised-but-invalid
  // value resolves to null there. A transport error rejects the promise; the
  // card swallows it and falls back to manual entry rather than showing an
  // error badge (discovery is best-effort, not operator action).
  //
  // Fails if the discovery try/catch is removed: the rejection propagates to
  // the outer catch and the card renders an error badge instead of a clean
  // manual-entry state.

  const pubkey = "3".repeat(64);

  setIpcHandler("get_admin_origin", () => Promise.resolve(null));
  setIpcHandler("admin_discover_origin", () =>
    Promise.reject(new Error("relay unreachable: network error")),
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
    `no probe must fire when discovery errors; got: ${JSON.stringify(probeOrigins)}`,
  );
  const input = container.querySelector("[data-testid='admin-origin-input']");
  assert.equal(
    input?.value,
    "",
    `input must be empty for manual entry after a discovery error; got: "${input?.value}"`,
  );
  const text = container.textContent ?? "";
  assert.ok(
    !text.includes("network error"),
    `a best-effort discovery error must not surface as an error badge; got: ${text.slice(0, 200)}`,
  );

  await unmount();
});

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
    return Promise.resolve("http://127.0.0.1:3000");
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

// ── community grouping ────────────────────────────────────────────────────

test("reports-grouped-by-community: multi-community reports render per-community headings", async () => {
  // The admin API returns deployment-wide reports; the console buckets them
  // by community for triage. Two communities → two group headings; rows stay
  // navigable (the first non-tab, non-processing report opens its detail).
  //
  // Mutation evidence: revert ReportsTab to a flat <ul> → community-group
  // headings vanish and this test goes red.

  const origin = "https://admin.example.com";
  const pubkey = "a7".repeat(32);

  const reports = [
    {
      id: "00000000-0000-0000-0000-0000000000a1",
      communityId: "comm-1",
      communityHost: "alpha.example.com",
      reportEventId: "aa",
      reporterPubkey: "bb",
      targetKind: "event",
      target: "cc",
      reportType: "spam",
      status: "open",
      createdAt: "2024-06-01T12:00:00Z",
    },
    {
      id: "00000000-0000-0000-0000-0000000000a2",
      communityId: "comm-2",
      communityHost: "beta.example.com",
      reportEventId: "dd",
      reporterPubkey: "ee",
      targetKind: "event",
      target: "ff",
      reportType: "abuse",
      status: "open",
      createdAt: "2024-06-02T12:00:00Z",
    },
  ];

  setIpcHandler("admin_list_reports", () => Promise.resolve(reports));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  const groups = container.querySelectorAll("[data-testid='community-group']");
  assert.equal(
    groups.length,
    2,
    `two communities must render two groups; got ${groups.length}`,
  );

  const hosts = Array.from(
    container.querySelectorAll("[data-testid='community-group-host']"),
  ).map((el) => el.textContent);
  assert.deepEqual(
    hosts,
    ["alpha.example.com", "beta.example.com"],
    `group headings must show each community host in first-seen order; got: ${JSON.stringify(hosts)}`,
  );

  await unmount();
});

test("feedback-grouped-by-community: multi-community feedback renders per-community headings", async () => {
  // Same grouping contract for the Feedback tab.
  //
  // Mutation evidence: revert FeedbackTab to a flat <ul> → group headings
  // vanish and this test goes red.

  const origin = "https://admin.example.com";
  const pubkey = "b8".repeat(32);

  const feedback = [
    {
      id: "00000000-0000-0000-0000-0000000000b1",
      communityId: "comm-1",
      communityHost: "alpha.example.com",
      submitterPubkey: "sub1",
      category: "bug",
      bodySummary: "Alpha feedback body",
      status: "new",
      receivedAt: "2024-06-01T09:00:00Z",
    },
    {
      id: "00000000-0000-0000-0000-0000000000b2",
      communityId: "comm-2",
      communityHost: "beta.example.com",
      submitterPubkey: "sub2",
      category: "idea",
      bodySummary: "Beta feedback body",
      status: "new",
      receivedAt: "2024-06-02T09:00:00Z",
    },
  ];

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve(feedback));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Switch to the Feedback tab.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  const hosts = Array.from(
    container.querySelectorAll("[data-testid='community-group-host']"),
  ).map((el) => el.textContent);
  assert.deepEqual(
    hosts,
    ["alpha.example.com", "beta.example.com"],
    `feedback group headings must show each community host; got: ${JSON.stringify(hosts)}`,
  );

  await unmount();
});

test("feedback-status-honest: a reviewed detail reports reviewed, never defaulting to new", async () => {
  // Thufir finding 5 (desktop half): `status` is a required wire field. A
  // reviewed/archived entry must render its real status after reload, not be
  // silently presented as "new". The status control must also initialize its
  // selected state from the server value.
  //
  // Mutation evidence: reinstate `detailState.data.status ?? "new"` in
  // FeedbackDetail → a reviewed entry would still show, but re-adding the
  // absent-defaulting cast and feeding an entry with no status would present
  // it as new; here we assert the reviewed value round-trips and its button
  // is the active (default-variant) one.

  const origin = "https://admin.example.com";
  const pubkey = "d5".repeat(32);

  const reviewedSummary = {
    id: "00000000-0000-0000-0000-0000000000d5",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    submitterPubkey: "sub-reviewed",
    category: "bug",
    bodySummary: "Already-triaged feedback",
    status: "reviewed",
    receivedAt: "2024-06-01T09:00:00Z",
  };
  const reviewedDetail = {
    id: reviewedSummary.id,
    communityId: reviewedSummary.communityId,
    communityHost: reviewedSummary.communityHost,
    eventId: "revevent",
    submitterPubkey: reviewedSummary.submitterPubkey,
    category: "bug",
    body: "Already-triaged feedback full body",
    status: "reviewed",
    tags: [],
    eventCreatedAt: "2024-06-01T09:00:00Z",
    receivedAt: "2024-06-01T09:00:00Z",
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () =>
    Promise.resolve([reviewedSummary]),
  );
  setIpcHandler("admin_get_feedback", () => Promise.resolve(reviewedDetail));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Switch to the Feedback tab.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // List row shows the "reviewed" badge (status !== "new").
  const listText = container.textContent ?? "";
  assert.ok(
    listText.includes("reviewed"),
    `list row must show the reviewed badge; got: ${listText.slice(0, 400)}`,
  );

  // Navigate into the feedback detail.
  const listRow = Array.from(container.querySelectorAll("button")).find(
    (btn) =>
      !(btn.getAttribute("data-testid") ?? "").startsWith("admin-tab") &&
      btn.textContent?.includes("Already-triaged feedback"),
  );
  assert.ok(listRow, "feedback list row must be present");
  await act(async () => {
    fireEvent.click(listRow);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // The status control initializes from the server value: the "reviewed"
  // button is the active (default-variant) selection, not "new".
  const control = container.querySelector(
    "[data-testid='feedback-status-control']",
  );
  assert.ok(control, "feedback status control must render");
  const reviewedBtn = container.querySelector(
    "[data-testid='feedback-status-btn-reviewed']",
  );
  const newBtn = container.querySelector(
    "[data-testid='feedback-status-btn-new']",
  );
  assert.ok(reviewedBtn && newBtn, "status buttons must render");
  // The active status is styled with a ring highlight (see FeedbackStatusControl).
  assert.ok(
    (reviewedBtn.className ?? "").includes("ring-2"),
    `the reviewed button must be marked active; got className: ${reviewedBtn.className}`,
  );
  assert.ok(
    !(newBtn.className ?? "").includes("ring-2"),
    `the new button must NOT be active for a reviewed entry; got className: ${newBtn.className}`,
  );

  await unmount();
});

// ── reopen ────────────────────────────────────────────────────────────────

/**
 * Mount the panel, wait for the list, then click the first non-tab report row
 * to open its detail. Returns after the detail has settled.
 */
async function openFirstReportDetail(container) {
  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    return;
  }
  throw new Error("no navigable report row found");
}

test("reopen-form-gated-by-status: resolved report shows the reopen form, open report does not", async () => {
  // The reopen form must render only for terminal reports
  // (resolved | dismissed | escalated) and never for an open report — an open
  // report shows the resolve form instead.
  //
  // Mutation evidence: drop the `isReopenable` gate → the form renders for
  // open reports too and the second assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "c1".repeat(32);

  const resolvedItem = {
    id: "00000000-0000-0000-0000-0000000000c1",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "resolved",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const resolvedDetail = {
    ...resolvedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([resolvedItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(resolvedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='reopen-report-form']"),
    "reopen form must render for a resolved report",
  );
  assert.equal(
    container.querySelector("[data-testid='resolve-report-form']"),
    null,
    "resolve form must NOT render for a resolved report",
  );

  await unmount();
});

test("reopen-submit: calls admin_reopen_report with requestId+reason, toasts, and refreshes", async () => {
  // The reopen submit must POST {requestId, reason} to admin_reopen_report,
  // fire a success toast, and bump the resolve generation so the detail
  // reloads (verified here by a second admin_get_report call returning the
  // now-open report, which flips the UI to the resolve form).
  //
  // Mutation evidence: remove `onReopened()` → no reload, detail stays
  // resolved, and the resolve-form assertion goes red. Remove the toast →
  // capturedToasts assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "c2".repeat(32);

  const base = {
    id: "00000000-0000-0000-0000-0000000000c2",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const dismissedItem = { ...base, status: "dismissed" };
  const dismissedDetail = {
    ...dismissedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: null,
    message: null,
  };
  const openDetail = {
    ...base,
    status: "open",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([dismissedItem]));
  // First detail load: dismissed. After reopen, the generation bump reloads
  // and the report is now open.
  let detailCalls = 0;
  setIpcHandler("admin_get_report", () => {
    detailCalls += 1;
    return Promise.resolve(detailCalls === 1 ? dismissedDetail : openDetail);
  });
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  let reopenArgs = null;
  setIpcHandler("admin_reopen_report", (args) => {
    reopenArgs = args;
    return Promise.resolve({ status: "open" });
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // Type a reason.
  const reasonInput = container.querySelector(
    "[data-testid='reopen-reason-input']",
  );
  assert.ok(reasonInput, "reopen reason input must be present");
  await act(async () => {
    fireEvent.change(reasonInput, { target: { value: "new evidence" } });
    await new Promise((r) => setTimeout(r, 5));
  });

  // Submit.
  const submit = container.querySelector("[data-testid='reopen-submit-btn']");
  assert.ok(submit, "reopen submit button must be present");
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(reopenArgs, "admin_reopen_report must be invoked");
  assert.equal(reopenArgs.origin, origin, "origin must be forwarded");
  assert.equal(reopenArgs.id, base.id, "report id must be forwarded");
  assert.equal(
    reopenArgs.body?.reason,
    "new evidence",
    "reason must be forwarded in the body",
  );
  assert.ok(
    typeof reopenArgs.body?.requestId === "string" &&
      reopenArgs.body.requestId.length > 0,
    `requestId must be a non-empty string; got: ${JSON.stringify(reopenArgs.body?.requestId)}`,
  );

  assert.ok(
    capturedToasts.some((m) => m.toLowerCase().includes("reopen")),
    `a reopen success toast must fire; got: ${JSON.stringify(capturedToasts)}`,
  );

  // Refresh: detail reloaded (call 2) and the report is now open → resolve form.
  assert.ok(
    detailCalls >= 2,
    "detail must reload after reopen (generation bump)",
  );
  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "after reopen, the now-open report must show the resolve form",
  );

  await unmount();
});

test("reopen-enforced-copy: a report with an actionId warns enforcement is not reversed", async () => {
  // Reopen is re-triage only. When the report carries an actionId (enforcement
  // was applied), the copy must say the enforcement is not reversed.
  //
  // Mutation evidence: collapse the `wasEnforced` branch to the generic copy →
  // the "not reversed" wording for un-ban/un-timeout/restore disappears.

  const origin = "https://admin.example.com";
  const pubkey = "c3".repeat(32);

  const escalatedItem = {
    id: "00000000-0000-0000-0000-0000000000c3",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "pubkey",
    target: "cc",
    reportType: "abuse",
    status: "escalated",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const escalatedDetail = {
    ...escalatedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: "00000000-0000-0000-0000-0000000000ff",
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([escalatedItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(escalatedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  const form = container.querySelector("[data-testid='reopen-report-form']");
  assert.ok(form, "reopen form must render for an escalated report");
  const text = form.textContent ?? "";
  assert.ok(
    text.toLowerCase().includes("not reversed"),
    `enforced-report copy must state the action is not reversed; got: ${text}`,
  );

  await unmount();
});

test("reopen-409-preserves-requestId: a not-reopenable conflict reuses the same requestId on retry", async () => {
  // A 409 (report is not reopenable — e.g. it moved to processing) is an
  // idempotency-relevant failure: the same requestId must be reused on retry
  // so the relay can dedupe. A non-409 failure resets it.
  //
  // Mutation evidence: drop the 409 branch in the catch → requestId resets and
  // the two attempts carry different ids, going red.

  const origin = "https://admin.example.com";
  const pubkey = "c4".repeat(32);

  const resolvedItem = {
    id: "00000000-0000-0000-0000-0000000000c4",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "resolved",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const resolvedDetail = {
    ...resolvedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([resolvedItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(resolvedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const requestIds = [];
  setIpcHandler("admin_reopen_report", (args) => {
    requestIds.push(args?.body?.requestId);
    return Promise.reject(
      new Error("409 report is not reopenable (current status: processing)"),
    );
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  const submit = container.querySelector("[data-testid='reopen-submit-btn']");
  assert.ok(submit, "reopen submit button must be present");

  // First attempt → 409.
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 20));
  });
  // Second attempt → 409 again; requestId must be identical.
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 20));
  });

  assert.equal(requestIds.length, 2, "two reopen attempts must have been made");
  assert.equal(
    requestIds[0],
    requestIds[1],
    `requestId must be preserved across a 409 retry; got: ${JSON.stringify(requestIds)}`,
  );

  // No success toast on a 409.
  assert.ok(
    !capturedToasts.some((m) => m.toLowerCase().includes("reopen")),
    `no success toast on a 409; got: ${JSON.stringify(capturedToasts)}`,
  );
  // The error is surfaced via toast.error with the parsed relay message.
  assert.ok(
    capturedErrorToasts.some((m) => m.includes("not reopenable")),
    `the 409 error message must surface via toast.error; got: ${JSON.stringify(capturedErrorToasts)}`,
  );

  await unmount();
});

test("cancel-on-failed: a failed action offers Cancel, POSTs {actionId} to admin_cancel_report, and reloads to open", async () => {
  // Cancel-then-resolve is the only recovery from a failed enforcement. The
  // block offers Cancel on `status: "failed"`, fences it on the action id, and
  // on success the report returns to `open` — the detail reload then serves
  // activeAction: null and re-exposes the resolve form for a fresh attempt.
  //
  // Mutation evidence: revert handleCancel to the old resolve-with-dismiss
  // masquerade → admin_cancel_report is never called and cancelArgs stays null.
  // Restore the `!activeAction` gate on the resolve form → the reopened report
  // still carries no action here, so this test isolates the cancel wiring.

  const origin = "https://admin.example.com";
  const pubkey = "e5".repeat(32);

  const base = {
    id: "00000000-0000-0000-0000-0000000000e5",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const actionId = "00000000-0000-0000-0000-0000000000f1";
  const failedDetail = {
    ...base,
    status: "processing",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: actionId,
      requestId: "00000000-0000-0000-0000-0000000000f2",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "failed",
      reason: null,
      expiresAt: null,
      errorMessage: "adapter timeout",
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:05Z",
    },
    message: null,
  };
  const openDetail = {
    ...base,
    status: "open",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "processing" }]),
  );
  let detailCalls = 0;
  setIpcHandler("admin_get_report", () => {
    detailCalls += 1;
    return Promise.resolve(detailCalls === 1 ? failedDetail : openDetail);
  });
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  let cancelArgs = null;
  setIpcHandler("admin_cancel_report", (args) => {
    cancelArgs = args;
    return Promise.resolve({
      status: "open",
      activeAction: { ...failedDetail.activeAction, status: "cancelled" },
    });
  });
  // The dismiss-masquerade path must be gone: resolve must never be called.
  let resolveCalled = false;
  setIpcHandler("admin_resolve_report", () => {
    resolveCalled = true;
    return Promise.reject(new Error("resolve must not be called by cancel"));
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // The failed action surfaces the error message and a single Cancel button.
  const block = container.querySelector(
    "[data-testid='enforcement-state-block']",
  );
  assert.ok(block, "enforcement-state-block must render for a failed action");
  assert.ok(
    (block.textContent ?? "").includes("adapter timeout"),
    `the failure errorMessage must render; got: ${block.textContent}`,
  );
  assert.equal(
    container.querySelector("[data-testid='enforcement-retry-btn']"),
    null,
    "the composed-retry button must be gone (Cancel-only on failed)",
  );
  const cancelBtn = container.querySelector(
    "[data-testid='enforcement-cancel-btn']",
  );
  assert.ok(cancelBtn, "the Cancel button must render on a failed action");

  await act(async () => {
    fireEvent.click(cancelBtn);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(cancelArgs, "admin_cancel_report must be invoked");
  assert.equal(cancelArgs.origin, origin, "origin must be forwarded");
  assert.equal(cancelArgs.id, base.id, "report id must be forwarded");
  assert.equal(
    cancelArgs.body?.actionId,
    actionId,
    "cancel must be fenced on the observed action id",
  );
  assert.equal(
    resolveCalled,
    false,
    "cancel must NOT go through the resolve endpoint (no dismiss masquerade)",
  );
  assert.ok(
    capturedToasts.some((m) => m.toLowerCase().includes("cancel")),
    `a cancel success toast must fire; got: ${JSON.stringify(capturedToasts)}`,
  );
  // Detail reloaded; the now-open report shows the resolve form for re-triage.
  assert.ok(detailCalls >= 2, "detail must reload after cancel");
  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "after cancel the reopened report must show the resolve form",
  );

  await unmount();
});

test("no-cancel-on-in-flight: pending and enforcing actions offer no cancel button", async () => {
  // Only a pre-mutation `failed` action is cancellable over HTTP. A stuck
  // `pending`/`enforcing` action is owned by the relay's recovery worker; the
  // UI must not offer a button that 409s by design.
  //
  // Mutation evidence: change the button gate from `=== "failed"` to include
  // enforcing → the assertion that no cancel button renders goes red.

  const origin = "https://admin.example.com";
  const pubkey = "e6".repeat(32);

  const base = {
    id: "00000000-0000-0000-0000-0000000000e6",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const enforcingDetail = {
    ...base,
    status: "processing",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000f3",
      requestId: "00000000-0000-0000-0000-0000000000f4",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "enforcing",
      reason: null,
      expiresAt: null,
      errorMessage: null,
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:01Z",
    },
    message: null,
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "processing" }]),
  );
  setIpcHandler("admin_get_report", () => Promise.resolve(enforcingDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='enforcement-state-block']"),
    "enforcement-state-block must render for an enforcing action",
  );
  assert.equal(
    container.querySelector("[data-testid='enforcement-cancel-btn']"),
    null,
    "no cancel button on an in-flight (enforcing) action",
  );
  // And the resolve form must stay suppressed on a processing report.
  assert.equal(
    container.querySelector("[data-testid='resolve-report-form']"),
    null,
    "resolve form must not render on a processing report",
  );

  await unmount();
});

test("reopened-after-enforcement: an open report carrying a succeeded action shows both history and the resolve form", async () => {
  // Honest history: a report enforced then reopened is `open` yet the detail
  // LATERAL still returns the succeeded action (the ban actually ran — a later
  // reopen does not un-happen it). The UI must render that action as executed
  // history AND still offer the resolve form, because the report is open for
  // re-triage. Cancel must NOT appear — cancel is failed-only.
  //
  // Mutation evidence: restore the `isOpen && !activeAction` gate → the resolve
  // form vanishes on this report and the operator is stranded, going red.

  const origin = "https://admin.example.com";
  const pubkey = "e7".repeat(32);

  const reopenedDetail = {
    id: "00000000-0000-0000-0000-0000000000e7",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000f5",
      requestId: "00000000-0000-0000-0000-0000000000f6",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "succeeded",
      reason: "confirmed spam",
      expiresAt: null,
      errorMessage: null,
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:03Z",
    },
    message: null,
    createdAt: "2024-06-01T11:00:00Z",
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...reopenedDetail, activeAction: undefined }]),
  );
  setIpcHandler("admin_get_report", () => Promise.resolve(reopenedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // Executed-enforcement history renders.
  const block = container.querySelector(
    "[data-testid='enforcement-state-block']",
  );
  assert.ok(block, "the succeeded action must render as enforcement history");
  assert.ok(
    (block.textContent ?? "").toLowerCase().includes("succeeded"),
    `history must show the succeeded state; got: ${block.textContent}`,
  );
  // Cancel is failed-only — never on a succeeded action.
  assert.equal(
    container.querySelector("[data-testid='enforcement-cancel-btn']"),
    null,
    "no cancel button on a succeeded action",
  );
  // The resolve form must still show — the report is open for re-triage.
  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "an open reopened-after-enforcement report must still show the resolve form",
  );

  await unmount();
});

test("feedback-severed-community: a purged-source feedback row renders in list and detail without its community", async () => {
  // Item 5 (desktop): feedback whose source community was purged carries a
  // null communityId/communityHost (tenant provenance severed, row retained as
  // operator evidence). The list must still render it (grouped under a
  // "source community removed" bucket) and the detail must show em-dashes for
  // the absent community fields — never crash on the null.
  //
  // Mutation evidence: narrow AdminFeedbackDto.communityId back to `string` →
  // typecheck breaks; restore the `communityId: string` grouping constraint →
  // the null key throws in groupByCommunity.

  const origin = "https://admin.example.com";
  const pubkey = "e8".repeat(32);

  const severedSummary = {
    id: "00000000-0000-0000-0000-0000000000e8",
    communityId: null,
    communityHost: null,
    submitterPubkey: "sub-severed",
    category: "bug",
    bodySummary: "Feedback from a since-purged community",
    status: "new",
    receivedAt: "2024-06-01T09:00:00Z",
  };
  const severedDetail = {
    id: severedSummary.id,
    communityId: null,
    communityHost: null,
    eventId: "sevevent",
    submitterPubkey: severedSummary.submitterPubkey,
    category: "bug",
    body: "Feedback from a since-purged community — full body",
    status: "new",
    tags: [],
    eventCreatedAt: "2024-06-01T09:00:00Z",
    receivedAt: "2024-06-01T09:00:00Z",
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([severedSummary]));
  setIpcHandler("admin_get_feedback", () => Promise.resolve(severedDetail));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // The severed row still renders in the list (did not throw / vanish).
  const listRow = Array.from(container.querySelectorAll("button")).find(
    (btn) =>
      !(btn.getAttribute("data-testid") ?? "").startsWith("admin-tab") &&
      btn.textContent?.includes("since-purged community"),
  );
  assert.ok(listRow, "the severed feedback row must render in the list");

  await act(async () => {
    fireEvent.click(listRow);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // Detail renders; the community fields show the em-dash placeholder.
  const fields = container.querySelector(
    "[data-testid='feedback-detail-fields']",
  );
  assert.ok(fields, "feedback detail must render for a severed row");
  assert.ok(
    (fields.textContent ?? "").includes("—"),
    `absent community fields must render as em-dash; got: ${fields.textContent}`,
  );

  await unmount();
});

// ── D3a: kick suppressed when the report carries no channel ────────────────

test("kick-suppressed-when-channel-null: an event report without a channel hides the Kick action", async () => {
  // Kick removes the target from the report's associated channel, so the relay
  // 400s (invalid_action_for_target) when the report has no channelId. The
  // resolve form must not offer an action guaranteed to fail. Other event
  // actions (ban/timeout/dismiss/delete/escalate) stay available.
  //
  // Mutation evidence: drop the `.filter((a) => a !== "kick" || channelId
  // != null)` guard → action-btn-kick renders and the null-channel assertion
  // goes red.

  const origin = "https://admin.example.com";
  const pubkey = "d3".repeat(32);

  const item = {
    id: "00000000-0000-0000-0000-0000000000d3",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const detail = {
    ...item,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([item]));
  setIpcHandler("admin_get_report", () => Promise.resolve(detail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "resolve form must render for an open report",
  );
  assert.equal(
    container.querySelector("[data-testid='action-btn-kick']"),
    null,
    "Kick must be suppressed when the report has no channelId",
  );
  // Sibling event actions remain available — only Kick is gated.
  assert.ok(
    container.querySelector("[data-testid='action-btn-ban']"),
    "Ban must still be offered on an event report",
  );

  await unmount();
});

test("kick-offered-when-channel-set: an event report with a channel offers the Kick action", async () => {
  // The paired case: when the report carries a channelId, Kick is a valid
  // action (the relay can enforce it) and must be offered.

  const origin = "https://admin.example.com";
  const pubkey = "d4".repeat(32);

  const item = {
    id: "00000000-0000-0000-0000-0000000000d4",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const detail = {
    ...item,
    channelId: "00000000-0000-0000-0000-0000000000ff",
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([item]));
  setIpcHandler("admin_get_report", () => Promise.resolve(detail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='action-btn-kick']"),
    "Kick must be offered when the report carries a channelId",
  );

  await unmount();
});

// ── D2: lists refetch on back-nav after a mutation ─────────────────────────

test("reports-list-refetches-on-back-after-mutation: resolving a report then navigating back shows fresh list status", async () => {
  // A mutation in the detail bumps a list generation fence propagated to the
  // ReportsTab, so returning to the list refetches instead of serving the
  // stale cached rows (Will's tab-switch workaround). Evidence is a second
  // admin_list_reports call after back-nav returning the updated status.
  //
  // Mutation evidence: drop the onMutated → setListGen wiring → the list
  // query key never changes, admin_list_reports is called once, and the
  // second-call assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "d5".repeat(32);

  const openItem = {
    id: "00000000-0000-0000-0000-0000000000d5",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "pubkey",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const openDetail = {
    ...openItem,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  // The list returns "open" first, then "dismissed" after the mutation — the
  // refetch must surface the new status.
  let listCalls = 0;
  setIpcHandler("admin_list_reports", () => {
    listCalls += 1;
    return Promise.resolve([
      { ...openItem, status: listCalls === 1 ? "open" : "dismissed" },
    ]);
  });
  setIpcHandler("admin_get_report", () => Promise.resolve(openDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_resolve_report", () =>
    Promise.resolve({ status: "dismissed" }),
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  await openFirstReportDetail(container);
  await settle(20);
  const callsBeforeBack = listCalls;

  // Dismiss the report.
  const dismissBtn = container.querySelector(
    "[data-testid='action-btn-dismiss']",
  );
  assert.ok(dismissBtn, "dismiss action must be present");
  await act(async () => {
    fireEvent.click(dismissBtn);
    await new Promise((r) => setTimeout(r, 10));
  });
  const submit = container.querySelector("[data-testid='resolve-submit-btn']");
  assert.ok(
    submit,
    "resolve submit button must appear after selecting dismiss",
  );
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  // Navigate back to the list.
  const backBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Back to reports"),
  );
  assert.ok(backBtn, "back-to-reports button must be present");
  await act(async () => {
    fireEvent.click(backBtn);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(
    listCalls > callsBeforeBack,
    `the list must refetch after back-nav following a mutation; before=${callsBeforeBack} after=${listCalls}`,
  );
  assert.ok(
    (container.textContent ?? "").includes("dismissed"),
    `the refetched list must show the updated status; got: ${(container.textContent ?? "").slice(0, 400)}`,
  );

  await unmount();
});

test("feedback-list-refetches-on-back-after-mutation: changing status then navigating back shows fresh list status", async () => {
  // Same fence for the Feedback tab: a status change in the detail bumps the
  // FeedbackTab list generation so back-nav refetches.
  //
  // Mutation evidence: drop the FeedbackDetail onMutated → setListGen wiring →
  // admin_list_feedback is called once and the second-call assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "d6".repeat(32);

  const summary = {
    id: "00000000-0000-0000-0000-0000000000d6",
    communityId: "00000000-0000-0000-0000-000000000022",
    communityHost: "relay.example.com",
    submitterPubkey: "submitter",
    category: "bug",
    bodySummary: "App crashes on startup",
    status: "new",
    receivedAt: "2024-05-01T09:00:05Z",
  };
  const detail = {
    id: summary.id,
    communityId: summary.communityId,
    communityHost: summary.communityHost,
    eventId: "feedevent",
    submitterPubkey: summary.submitterPubkey,
    category: "bug",
    body: "App crashes on startup — full detail",
    status: "new",
    tags: [],
    eventCreatedAt: "2024-05-01T09:00:00Z",
    receivedAt: "2024-05-01T09:00:05Z",
  };

  let listCalls = 0;
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => {
    listCalls += 1;
    return Promise.resolve([
      { ...summary, status: listCalls === 1 ? "new" : "reviewed" },
    ]);
  });
  setIpcHandler("admin_get_feedback", () => Promise.resolve(detail));
  setIpcHandler("admin_patch_feedback", () =>
    Promise.resolve({ status: "reviewed" }),
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Switch to the Feedback tab.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);
  assert.equal(listCalls, 1, "feedback list is fetched once on tab open");

  // Open the first feedback row.
  const row = Array.from(container.querySelectorAll("button")).find(
    (b) =>
      !(b.getAttribute("data-testid") ?? "").startsWith("admin-tab") &&
      b.textContent?.includes("App crashes"),
  );
  assert.ok(row, "feedback row must be present");
  await act(async () => {
    fireEvent.click(row);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  // Mark reviewed.
  const reviewedBtn = container.querySelector(
    "[data-testid='feedback-status-btn-reviewed']",
  );
  assert.ok(reviewedBtn, "reviewed status button must be present");
  await act(async () => {
    fireEvent.click(reviewedBtn);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  // Navigate back to the feedback list.
  const backBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Back to feedback"),
  );
  assert.ok(backBtn, "back-to-feedback button must be present");
  await act(async () => {
    fireEvent.click(backBtn);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(
    listCalls >= 2,
    `the feedback list must refetch after back-nav following a status change; listCalls=${listCalls}`,
  );
  assert.ok(
    (container.textContent ?? "").includes("reviewed"),
    `the refetched feedback list must show the updated status; got: ${(container.textContent ?? "").slice(0, 400)}`,
  );

  await unmount();
});

test("resolve-rejection-surfaces-parsed-message: a rejected resolve toasts the relay message, not raw JSON", async () => {
  // A resolve mutation that the relay rejects (e.g. invalid_action_for_target)
  // must surface the envelope's human message via toast.error — never the raw
  // JSON envelope and never a success toast.
  //
  // Mutation evidence: replace `toast.error(adminErrorMessage(e))` in
  // handleSubmit with `toast.error(String(e))` → the raw-JSON assertion goes
  // red because the envelope leaks verbatim.

  const origin = "https://admin.example.com";
  const pubkey = "f7".repeat(32);

  const openItem = {
    id: "00000000-0000-0000-0000-0000000000f7",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const openDetail = {
    ...openItem,
    channelId: "00000000-0000-0000-0000-0000000000ff",
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  const humanMessage =
    "action kick requires the report to have an associated channel";
  // The native command rejects with `admin API error: {envelope}` — exactly the
  // shape adminErrorMessage strips down to the envelope's `message`.
  const rawError = `admin API error: {"error":{"code":"invalid_action_for_target","message":"${humanMessage}","requestId":"00000000-0000-0000-0000-0000000000e7"}}`;

  setIpcHandler("admin_list_reports", () => Promise.resolve([openItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(openDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_resolve_report", () =>
    Promise.reject(new Error(rawError)),
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // Select the kick action, then submit — the relay rejects it.
  const kickBtn = container.querySelector("[data-testid='action-btn-kick']");
  assert.ok(kickBtn, "kick action must be present (channel is set)");
  await act(async () => {
    fireEvent.click(kickBtn);
    await new Promise((r) => setTimeout(r, 10));
  });
  const submit = container.querySelector("[data-testid='resolve-submit-btn']");
  assert.ok(submit, "resolve submit button must appear after selecting kick");
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  // The parsed human message reaches toast.error.
  assert.ok(
    capturedErrorToasts.some((m) => m.includes(humanMessage)),
    `the parsed relay message must surface via toast.error; got: ${JSON.stringify(capturedErrorToasts)}`,
  );
  // The raw JSON envelope must NOT leak into any error toast.
  assert.ok(
    !capturedErrorToasts.some(
      (m) => m.includes('{"error"') || m.includes("admin API error:"),
    ),
    `the raw JSON envelope must not appear in an error toast; got: ${JSON.stringify(capturedErrorToasts)}`,
  );
  // No success toast on a rejected resolve.
  assert.ok(
    !capturedToasts.some((m) => m.toLowerCase().includes("resolved")),
    `no success toast on a rejected resolve; got: ${JSON.stringify(capturedToasts)}`,
  );

  await unmount();
});
