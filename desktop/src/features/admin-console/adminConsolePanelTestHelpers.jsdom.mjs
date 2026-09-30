/**
 * Shared test infrastructure for AdminConsolePanel jsdom test suites.
 *
 * Exports the Tauri IPC interceptor, toast capture, and mount helpers used
 * across the per-tab jsdom test files. Imported as a side-effect by each
 * file via the module-singleton pattern (ES modules are cached).
 *
 * NOTE: This file does NOT register afterEach. Each test file that imports it
 * must register its own afterEach calling resetTestState() from this module.
 */
// ── Tauri IPC interceptor ────────────────────────────────────────────────────
//
// @tauri-apps/api/core calls `window.__TAURI_INTERNALS__.invoke(...)` where
// `window` is the jsdom window object (set via test-jsdom-setup.mjs), not
// `globalThis`. Both globalThis.__TAURI_INTERNALS__ and window.__TAURI_INTERNALS__
// must be set so all import paths reach the same mock.

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
export const ipcHandlers = new Map();

export function setIpcHandler(cmd, fn) {
  ipcHandlers.set(cmd, fn);
}
export const TEST_RELAY_WS_URL = "wss://relay.test";
export function clearIpcHandlers() {
  ipcHandlers.clear();
  // The restrictions list captures the native relay it loads from.
  ipcHandlers.set("get_relay_ws_url", () => Promise.resolve(TEST_RELAY_WS_URL));
}
clearIpcHandlers();

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
import { CommunitiesProvider } from "@/features/communities/useCommunities.tsx";

// ── Success-toast capture ────────────────────────────────────────────────────
//
// sonner's `toast` is a shared singleton object across import paths (verified),
// so replacing `toast.success` here is observed by the production components.
// Captured messages are asserted by the toast tests and cleared in afterEach.

/** @type {string[]} */
export const capturedToasts = [];
toast.success = (msg) => {
  capturedToasts.push(String(msg));
  return 0;
};

/** @type {string[]} */
export const capturedErrorToasts = [];
toast.error = (msg) => {
  capturedErrorToasts.push(String(msg));
  return 0;
};

// Reset state between tests — each test file registers its own afterEach
// that calls this function.
export function resetTestState() {
  clearIpcHandlers();
  capturedToasts.length = 0;
  capturedErrorToasts.length = 0;
}

// ── Typed native mutation error ──────────────────────────────────────────────
//
// Admin mutation commands reject with a serialized Rust `AdminMutationError`
// (`{message, relayStatus, bodyComplete}`, camelCase). The real tauri bridge
// rejects with that plain object and `toTauriError` wraps it into a
// `TauriInvokeError` whose `.message` is the message and `.payload` is the
// whole object — from which the UI reads `relayStatus`/`bodyComplete` to decide
// idempotency-retry policy. Rejecting with a plain object here (NOT an Error)
// reproduces that wire shape exactly.
//
// `relayStatus` is a number when the relay authoritatively answered, and `null`
// for a transport/pre-send failure where no relay verdict exists. `bodyComplete`
// is true only when the relay's full body was read; it defaults to `relayStatus
// !== null` (a status with a fully-read body — the common authoritative case),
// and callers pass `false` explicitly to model a truncated/lost-body response.
export function mutationReject(
  message,
  relayStatus,
  bodyComplete = relayStatus !== null,
) {
  return Promise.reject({ message, relayStatus, bodyComplete });
}

// ── Deferred promise helper ──────────────────────────────────────────────────

export function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// ── Mount helpers ────────────────────────────────────────────────────────────

export function makeQueryClient(pubkeyHex) {
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

// ReportsTab (the panel's default tab) and StaffingTab both resolve profile
// names via useUsersBatchQuery, which needs CommunitiesProvider and a
// get_users_batch handler.
export function mountCard(qc) {
  if (!ipcHandlers.get("get_users_batch")) {
    setIpcHandler("get_users_batch", () =>
      Promise.resolve({ profiles: {}, missing: [] }),
    );
  }
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const doRender = async () => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(AdminConsoleSettingsCard),
          ),
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

export function mountPanel({
  origin,
  pubkey,
  canMutate = true,
  role = undefined,
  initialTab = undefined,
  onSelfMutation = undefined,
}) {
  const qc = makeQueryClient(pubkey);
  // StaffingTab calls useUsersBatchQuery which needs QueryClientProvider +
  // CommunitiesProvider. Provide a default no-op handler so profile lookups
  // resolve without error when individual tests don't override get_users_batch.
  if (!ipcHandlers.get("get_users_batch")) {
    setIpcHandler("get_users_batch", () =>
      Promise.resolve({ profiles: {}, missing: [] }),
    );
  }
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const doRender = async ({ origin: o, pubkey: p } = { origin, pubkey }) => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(AdminConsolePanel, {
              canMutate,
              origin: o,
              pubkey: p,
              ...(role !== undefined ? { role } : {}),
              ...(initialTab !== undefined ? { initialTab } : {}),
              ...(onSelfMutation !== undefined ? { onSelfMutation } : {}),
            }),
          ),
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

// makeOpenReportFixtures — build a standard open-report list/detail pair and register
// the matching admin_list_reports / admin_get_report / admin_list_feedback handlers.
// Returns {openItem, openDetail} for tests that need to reference the fixtures directly.
// `itemOverrides` may patch any list-item fields (e.g. targetKind/target/id).
export function makeOpenReportFixtures(id, itemOverrides = {}) {
  const openItem = {
    id,
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-07-01T00:00:00Z",
    ...itemOverrides,
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
  setIpcHandler("admin_list_reports", () => Promise.resolve([openItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(openDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  return { openItem, openDetail };
}

// mountStaffingPanel — convenience wrapper for tests that mount AdminConsolePanel
// in staffing-tab operator mode with standard empty-reports list handlers.
// Mutation handlers (admin_put_operator / admin_delete_operator) are set by the
// individual test BEFORE calling this helper; list handlers are set here.
export function mountStaffingPanel(
  origin,
  pubkey,
  operators = [],
  { onSelfMutation } = {},
) {
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve(operators.map((op) => ({ ...op }))),
  );
  return mountPanel({
    origin,
    pubkey,
    canMutate: true,
    role: "operator",
    initialTab: "staffing",
    ...(onSelfMutation !== undefined ? { onSelfMutation } : {}),
  });
}

export async function settle(ms = 20) {
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

// ── canMutate-false shared fixtures ─────────────────────────────────────────
//
// Carl finding P2-1: "every mutation affordance in the panel" must be gated
// on canMutate. Factories below build fresh fixture objects per test call.

/** Shared canMutate=false constants (origin, acting pubkey, op pubkey). */
export const CM_ORIGIN = "https://admin-readonly.example.com";
export const CM_PUBKEY = "cc".repeat(32);
export const CM_OP_PUBKEY = "dd".repeat(32);

/** Build feedback summary/detail fixtures for canMutate-false tests. */
export function makeCmFalseFeedback() {
  const feedbackSummary = {
    id: "00000000-0000-0000-0000-000000000099",
    communityId: "comm-1",
    communityHost: "relay.example.com",
    submitterPubkey: "sub001",
    category: null,
    bodySummary: "readonly feedback",
    receivedAt: "2024-01-01T00:00:00Z",
  };
  const feedbackDetail = {
    id: "00000000-0000-0000-0000-000000000099",
    communityId: "comm-1",
    communityHost: "relay.example.com",
    eventId: "fev001",
    submitterPubkey: "sub001",
    category: null,
    body: "readonly feedback full",
    status: "new",
    tags: [],
    eventCreatedAt: "2024-01-01T00:00:00Z",
    receivedAt: "2024-01-01T00:00:00Z",
  };
  return { feedbackSummary, feedbackDetail };
}

// Re-export act and fireEvent so tab files don't need separate imports for them.
export {
  React,
  act,
  fireEvent,
  createRoot,
  QueryClient,
  QueryClientProvider,
  CommunitiesProvider,
  AdminConsoleSettingsCard,
  AdminConsolePanel,
};
