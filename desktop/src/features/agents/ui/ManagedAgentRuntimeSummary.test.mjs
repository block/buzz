import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ManagedAgentRuntimeSummary } from "./ManagedAgentRuntimeSummary.tsx";

function runtime(overrides = {}) {
  return {
    pubkey: "aa",
    relayUrl: "wss://relay.example",
    localSetup: true,
    lifecycle: "ready",
    pid: 42,
    error: null,
    logPath: null,
    activeAssignment: null,
    activeJob: null,
    ...overrides,
  };
}

test("migration-blocked runtime states render explicit operator guidance", () => {
  const legacy = renderToStaticMarkup(
    React.createElement(ManagedAgentRuntimeSummary, {
      runtime: runtime({ lifecycle: "legacy_runtime_active" }),
    }),
  );
  const manual = renderToStaticMarkup(
    React.createElement(ManagedAgentRuntimeSummary, {
      runtime: runtime({ lifecycle: "manual_legacy_stop_required" }),
    }),
  );

  assert.match(legacy, /Legacy runtime active/);
  assert.match(legacy, /Stop the legacy runtime/);
  assert.match(manual, /Manual stop required/);
  assert.match(manual, /cannot be verified safely/);
});

test("active assignment, job and publication failure are visible together", () => {
  const html = renderToStaticMarkup(
    React.createElement(ManagedAgentRuntimeSummary, {
      runtime: runtime({
        lifecycle: "recovering",
        activeAssignment: {
          assignmentId: "assignment-1",
          channelId: "channel-1",
          sourceEventId: "event-1",
          state: "blocked",
          summary: "Repair JAC-575",
          activeJobId: "job-1",
          lastProgressAt: "2026-08-02T10:00:00Z",
          hasBlocker: true,
        },
        activeJob: {
          jobId: "job-1",
          requestEventId: "request-1",
          sourceEventId: "event-1",
          channelId: "channel-1",
          state: "running",
          attempt: 2,
          progressSeq: 7,
          summary: "Receipt verification",
          startedAt: "2026-08-02T09:59:00Z",
          finishedAt: null,
          exitCode: null,
          errorCode: null,
          publicationState: "failed",
          runnerPid: null,
          runnerStartMarker: null,
        },
      }),
    }),
  );

  assert.match(html, /Recovering/);
  assert.match(html, /Repair JAC-575/);
  assert.match(html, /blocked · job job-1/);
  assert.match(html, /Blocked — see the source thread for blocker details/);
  assert.match(html, /Receipt verification/);
  assert.match(html, /Progress update 7 · attempt 2/);
  assert.match(html, /relay failed/);
  assert.match(html, /Source thread/);
  assert.match(html, /buzz:\/\/message\?channel=channel-1&amp;id=event-1/);
  assert.match(html, /latest relay publication failed/);
});

test("approval state remains visible without an active turn or job", () => {
  const html = renderToStaticMarkup(
    React.createElement(ManagedAgentRuntimeSummary, {
      runtime: runtime({
        activeAssignment: {
          assignmentId: "assignment-approval",
          channelId: "channel-approval",
          sourceEventId: "event-approval",
          state: "needs_approval",
          summary: "Apply protected release",
          activeJobId: null,
          lastProgressAt: "2026-08-02T10:00:00Z",
          hasBlocker: false,
        },
      }),
    }),
  );

  assert.match(html, /Apply protected release/);
  assert.match(html, /needs approval/);
  assert.match(html, /Approval required before work can continue/);
  assert.match(html, /Source thread/);
});
