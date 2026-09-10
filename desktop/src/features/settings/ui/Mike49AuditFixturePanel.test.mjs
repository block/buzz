/**
 * MIKE-49 checkpoint 1: component-level proof of every UI state the
 * fixture-only audit panel must handle — loading, success (with both a
 * non-empty capped/incomplete pagination result and a genuinely empty
 * fixture-source probe surfaced), failure, and the duplicate-run guard.
 * `mike49RunFixtureAudit`'s underlying Tauri IPC is stubbed directly (same
 * `__TAURI_INTERNALS__.invoke` convention as
 * `features/agents/ui/effortAutoClear.test.mjs`), so this never launches
 * the real Tauri app or touches a real command — see this repo's own
 * README note that a full manual/visual verification of the running app is
 * still separately recommended.
 */

import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  document: dom.window.document,
  window: dom.window,
  IS_REACT_ACT_ENVIRONMENT: true,
  HTMLElement: dom.window.HTMLElement,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
dom.window.matchMedia ??= (query) => ({
  matches: false,
  media: query,
  addEventListener: () => {},
  removeEventListener: () => {},
  addListener: () => {},
  removeListener: () => {},
  dispatchEvent: () => false,
});
globalThis.matchMedia = dom.window.matchMedia;

// ── Tauri IPC stub ───────────────────────────────────────────────────────────
// Mutable per-test so each test controls exactly how `mike49_run_fixture_audit`
// resolves/rejects, and how many times it was actually invoked (proves the
// duplicate-run guard).
let invokeHandler = null;
let invokeCallCount = 0;

globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd, payload) => {
    invokeCallCount += 1;
    if (cmd === "mike49_run_fixture_audit" && invokeHandler) {
      return invokeHandler(payload);
    }
    return Promise.reject(new Error(`unmocked: ${cmd}`));
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

let act, render, screen, cleanup, fireEvent;
let createElement;
let Mike49AuditFixturePanel;

before(async () => {
  ({ act, render, screen, cleanup, fireEvent } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ Mike49AuditFixturePanel } = await import(
    "./Mike49AuditFixturePanel.tsx"
  ));
});

afterEach(() => {
  cleanup?.();
  invokeHandler = null;
  invokeCallCount = 0;
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const CAPPED_REPORT = {
  simulated: true,
  dataSource: "in_memory_fixture",
  records: [
    {
      scenario: "wrong_recipient_verify_error",
      result: "verify_error",
      verifyErrorCode: "wrong_recipient",
      observeOutcomeCode: null,
      eventId: null,
      signer: null,
      sessionId: null,
      turnId: null,
      turnSeq: null,
      harness: null,
      model: null,
      stopReason: null,
      tokens: null,
      cost: null,
    },
    {
      scenario: "accepted_record",
      result: "verified",
      verifyErrorCode: null,
      observeOutcomeCode: "accepted",
      eventId: "a".repeat(64),
      signer: "b".repeat(64),
      sessionId: "mike49-fixture-session-accepted",
      turnId: "turn-1",
      turnSeq: 1,
      harness: "codex-acp",
      model: "gpt-5",
      stopReason: "end_turn",
      tokens: {
        turnInputTokens: 100,
        turnOutputTokens: 10,
        turnTotalTokens: 110,
        cumulativeInputTokens: 100,
        cumulativeOutputTokens: 10,
        cumulativeTotalTokens: 110,
        deltaReliable: true,
      },
      cost: {
        turnCostUsd: 0.01,
        cumulativeCostUsd: 0.01,
        note: "harness estimate, never a billed charge",
      },
    },
  ],
  pagination: {
    eventsExamined: 2,
    stopReason: "page_cap_reached",
    localTraversalComplete: false,
    pageLimit: 2,
    maxPagesPerCall: 1,
    note: "local_traversal_complete only means this run's own fixture page source had nothing further to return",
  },
  emptySourceProbe: {
    eventsExamined: 0,
    stopReason: "exhausted",
    note: "a genuinely empty in-memory fixture page source",
  },
  sanitizer: { blockedCount: 0, blockedByKind: {}, droppedFieldCount: 0 },
};

test("idle: renders the run button and never auto-invokes on mount", async () => {
  invokeHandler = () => Promise.resolve(CAPPED_REPORT);
  render(createElement(Mike49AuditFixturePanel));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(invokeCallCount, 0, "must not invoke until the button is clicked");
  assert.ok(screen.getByTestId("mike49-run-button"));
  assert.equal(screen.queryByTestId("mike49-report"), null);
});

test("loading: shows a loading indicator and disables the button while in flight", async () => {
  const gate = deferred();
  invokeHandler = () => gate.promise;
  render(createElement(Mike49AuditFixturePanel));

  await act(async () => {
    fireEvent.click(screen.getByTestId("mike49-run-button"));
  });

  assert.ok(screen.getByTestId("mike49-loading"), "loading state must render");
  assert.equal(
    screen.getByTestId("mike49-run-button").disabled,
    true,
    "the run button must be disabled while a run is in flight",
  );

  await act(async () => {
    gate.resolve(CAPPED_REPORT);
    await gate.promise;
  });
});

test("success: renders records, the capped/incomplete pagination state, and the empty-source probe", async () => {
  invokeHandler = () => Promise.resolve(CAPPED_REPORT);
  render(createElement(Mike49AuditFixturePanel));

  await act(async () => {
    fireEvent.click(screen.getByTestId("mike49-run-button"));
  });

  assert.ok(screen.getByTestId("mike49-report"));
  const rows = screen.getAllByTestId("mike49-record-row");
  assert.equal(rows.length, 2, "both fetched records must render");
  assert.match(screen.getByTestId("mike49-pagination-state").textContent, /Incomplete/);
  assert.ok(
    screen.getByTestId("mike49-empty-probe").textContent.includes("0 events"),
    "the empty-source probe must be visibly distinct from the capped/incomplete records list",
  );
});

test("empty records: renders the empty state instead of an empty list with no explanation", async () => {
  invokeHandler = () =>
    Promise.resolve({
      ...CAPPED_REPORT,
      records: [],
      pagination: { ...CAPPED_REPORT.pagination, eventsExamined: 0, stopReason: "exhausted", localTraversalComplete: true },
    });
  render(createElement(Mike49AuditFixturePanel));

  await act(async () => {
    fireEvent.click(screen.getByTestId("mike49-run-button"));
  });

  assert.ok(screen.getByTestId("mike49-records-empty"));
  assert.equal(screen.queryAllByTestId("mike49-record-row").length, 0);
});

test("failure: shows a failure state with a fixed, content-free error description", async () => {
  invokeHandler = () => Promise.reject(new Error("reconciliation_failed"));
  render(createElement(Mike49AuditFixturePanel));

  await act(async () => {
    fireEvent.click(screen.getByTestId("mike49-run-button"));
  });

  const errorEl = screen.getByTestId("mike49-error");
  assert.ok(errorEl);
  assert.match(errorEl.textContent, /did not reconcile/);
});

test("duplicate-run guard: a second click while a run is in flight does not start a second invoke", async () => {
  const gate = deferred();
  invokeHandler = () => gate.promise;
  render(createElement(Mike49AuditFixturePanel));

  await act(async () => {
    fireEvent.click(screen.getByTestId("mike49-run-button"));
  });
  await act(async () => {
    fireEvent.click(screen.getByTestId("mike49-run-button"));
    fireEvent.click(screen.getByTestId("mike49-run-button"));
  });

  assert.equal(
    invokeCallCount,
    1,
    "extra clicks while a run is in flight must not dispatch another invoke",
  );

  await act(async () => {
    gate.resolve(CAPPED_REPORT);
    await gate.promise;
  });
});
