import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { parseJobResultContent } from "../lib/jobResult.ts";
import { JobResultCard } from "./JobResultCard.tsx";

const JOB_REQUEST =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

function parse(overrides = {}) {
  const result = parseJobResultContent(
    JSON.stringify({
      schemaVersion: 1,
      jobRequest: JOB_REQUEST,
      requestedOutcome: "Make the result inspectable",
      outcome: "The handoff is ready.",
      lastProgress: "Full verification passed.",
      disposition: "completed",
      artifacts: [
        {
          kind: "pull_request",
          label: "Pull request",
          reference: "https://github.com/block/buzz/pull/1",
          sourceState: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        },
      ],
      verification: [
        {
          label: "just ci",
          status: "passed",
          evidence: "exit 0",
        },
      ],
      ...overrides,
    }),
    JOB_REQUEST,
  );
  assert.ok(result);
  return result;
}

test("renders the disposition, outcome, artifact, and verification evidence", () => {
  const html = renderToStaticMarkup(
    React.createElement(JobResultCard, { result: parse() }),
  );

  assert.match(html, /data-disposition="completed"/);
  assert.match(html, />Completed</);
  assert.match(html, /The handoff is ready\./);
  assert.match(html, /Make the result inspectable/);
  assert.match(html, /Full verification passed\./);
  assert.match(html, /Pull request/);
  assert.match(html, /href="https:\/\/github\.com\/block\/buzz\/pull\/1"/);
  assert.match(html, /just ci/);
  assert.match(html, />Passed</);
});

test("renders blockers and failed verification without hiding the evidence", () => {
  const html = renderToStaticMarkup(
    React.createElement(JobResultCard, {
      result: parse({
        disposition: "blocked",
        artifacts: [],
        blocker: "Maintainer decision required.",
        verification: [
          {
            label: "Desktop smoke",
            status: "failed",
            evidence: "Result card did not load.",
          },
        ],
      }),
    }),
  );

  assert.match(html, /data-disposition="blocked"/);
  assert.match(html, /Maintainer decision required\./);
  assert.match(html, /Desktop smoke/);
  assert.match(html, />Failed</);
  assert.match(html, /Result card did not load\./);
  assert.match(html, /No artifact was reported for this result\./);
  assert.doesNotMatch(html, /No durable artifact was expected/);
});

test("renders explicit empty states for a no-artifact handoff", () => {
  const html = renderToStaticMarkup(
    React.createElement(JobResultCard, {
      result: parse({
        disposition: "no_artifact",
        artifacts: [],
        verification: [],
        lastProgress: undefined,
      }),
    }),
  );

  assert.match(html, /data-disposition="no_artifact"/);
  assert.match(html, /Completed without an artifact/);
  assert.match(html, /No durable artifact was expected/);
  assert.match(html, /No verification was reported/);
});

test("does not turn non-http artifact references into links", () => {
  const html = renderToStaticMarkup(
    React.createElement(JobResultCard, {
      result: parse({
        artifacts: [
          {
            kind: "file",
            label: "Report",
            reference: "docs/report.md",
          },
        ],
      }),
    }),
  );

  assert.match(html, /docs\/report\.md/);
  assert.doesNotMatch(html, /href="docs\/report\.md"/);
});
