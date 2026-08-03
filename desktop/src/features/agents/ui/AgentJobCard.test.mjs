import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AgentJobCard } from "./AgentJobCard.tsx";

const JOB_ID = "123e4567-e89b-42d3-a456-426614174000";

function job(overrides = {}) {
  return {
    jobId: JOB_ID,
    requestEventId: "44".repeat(32),
    sourceEventId: "33".repeat(32),
    channelId: "36411e44-0e2d-4cfe-bd6e-567eb169db9f",
    state: "running",
    summary: "Running receipt verification",
    attempt: 1,
    progressSeq: 4,
    requestedAt: 1_700_000_000,
    startedAt: 1_700_000_001,
    finishedAt: null,
    exitCode: null,
    errorCode: null,
    artifacts: [],
    publicationFailed: false,
    eventIds: ["44".repeat(32), "55".repeat(32)],
    ...overrides,
  };
}

test("active card exposes status, elapsed time, source, artifact and enabled cancel seam", () => {
  const html = renderToStaticMarkup(
    React.createElement(AgentJobCard, {
      job: job({
        artifacts: [{ name: "receipt.json", uri: "artifact://receipt" }],
      }),
      nowMs: 1_700_000_011_000,
      onCancel() {},
    }),
  );

  assert.match(html, /Running receipt verification/);
  assert.match(html, />10s</);
  assert.match(html, /Source message/);
  assert.match(html, /artifact:\/\/receipt/);
  assert.match(html, new RegExp(`aria-label="Cancel job ${JOB_ID}"`));
  assert.doesNotMatch(html, /disabled=""/);
});

test("terminal result renders evidence and disables its accessible cancel action", () => {
  const html = renderToStaticMarkup(
    React.createElement(AgentJobCard, {
      job: job({
        state: "succeeded",
        summary: "Repair delivered",
        finishedAt: 1_700_000_031,
        exitCode: 0,
      }),
      nowMs: 1_700_000_100_000,
      onCancel() {},
    }),
  );

  assert.match(html, /Succeeded/);
  assert.match(html, /Completed · exit 0/);
  assert.match(html, /disabled=""/);
  assert.match(html, /title="This job is already finished"/);
  assert.match(html, new RegExp(`aria-label="Cancel job ${JOB_ID}"`));
});

test("failed terminal and publication failure remain visibly distinct", () => {
  const html = renderToStaticMarkup(
    React.createElement(AgentJobCard, {
      job: job({
        state: "failed",
        summary: "Runner failed",
        finishedAt: 1_700_000_021,
        errorCode: "runner_failed",
        publicationFailed: true,
      }),
      nowMs: 1_700_000_100_000,
      onCancel() {},
    }),
  );

  assert.match(html, /Failed · runner_failed/);
  assert.match(
    html,
    /Result saved locally, but its relay publication failed\./,
  );
  assert.match(html, /role="status"/);
});
