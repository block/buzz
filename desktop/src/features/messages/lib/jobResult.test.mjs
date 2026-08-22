import assert from "node:assert/strict";
import test from "node:test";

import {
  getJobArtifactKindLabel,
  getJobResultFeedHeadline,
  getJobResultFeedPresentation,
  getJobResultRequestId,
  parseJobResultContent,
} from "./jobResult.ts";

const JOB_REQUEST =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

function parseContent(content, expectedJobRequest = JOB_REQUEST) {
  return parseJobResultContent(content, expectedJobRequest);
}

function manifest(overrides = {}) {
  return JSON.stringify({
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
  });
}

test("parses a structured job result and ignores additive fields", () => {
  const result = parseContent(
    manifest({
      futureField: "ignored",
      artifacts: [
        {
          kind: "pull_request",
          label: "Pull request",
          reference: "https://github.com/block/buzz/pull/1",
          futureArtifactField: true,
        },
      ],
    }),
  );

  assert.equal(result?.disposition, "completed");
  assert.equal(result?.outcome, "The handoff is ready.");
  assert.equal(result?.artifacts[0]?.kind, "pull_request");
  assert.equal(result?.verification[0]?.status, "passed");
});

test("supports every artifact type and reader-facing label", () => {
  const kinds = [
    "file",
    "media",
    "branch",
    "commit",
    "pull_request",
    "canvas",
    "workflow_output",
    "build",
    "deployment",
    "link",
    "other",
  ];
  const references = {
    file: "docs/report.md",
    media: "https://example.com/report.png",
    branch: "agent/job-handoff",
    commit: "b".repeat(40),
    pull_request: "https://github.com/block/buzz/pull/1",
    canvas: "buzz://canvas?channel=aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    workflow_output: "run-123",
    build: "https://example.com/build/123",
    deployment: "https://example.com/deploy/123",
    link: "https://example.com/report",
    other: "signed-note-123",
  };
  const artifacts = kinds.map((kind) => ({
    kind,
    label: `${kind} artifact`,
    reference: references[kind],
  }));

  const result = parseContent(manifest({ artifacts }));

  assert.deepEqual(
    result?.artifacts.map((artifact) => artifact.kind),
    kinds,
  );
  assert.deepEqual(
    result?.artifacts.map((artifact) => getJobArtifactKindLabel(artifact.kind)),
    [
      "File",
      "Media",
      "Branch",
      "Commit",
      "Pull request",
      "Canvas",
      "Workflow output",
      "Build",
      "Deployment",
      "Link",
      "Artifact",
    ],
  );
});

test("parses an explicit no-artifact result", () => {
  const result = parseContent(
    manifest({
      disposition: "no_artifact",
      artifacts: [],
      verification: [],
      lastProgress: undefined,
    }),
  );

  assert.equal(result?.disposition, "no_artifact");
  assert.deepEqual(result?.artifacts, []);
  assert.equal(getJobResultFeedHeadline(result.disposition), "Job completed");
});

test("projects a human-readable Home feed headline and outcome", () => {
  assert.deepEqual(getJobResultFeedPresentation(manifest(), JOB_REQUEST), {
    headline: "Job completed",
    content: "The handoff is ready.",
  });
  assert.equal(
    getJobResultFeedPresentation("legacy plaintext result", JOB_REQUEST),
    null,
  );
});

test("rejects legacy text, malformed JSON, and unsupported schemas", () => {
  assert.equal(parseContent("Done."), null);
  assert.equal(parseContent("{not-json"), null);
  assert.equal(parseContent(manifest({ schemaVersion: 2 })), null);
});

test("rejects invalid enums and malformed item arrays", () => {
  assert.equal(parseContent(manifest({ disposition: "mostly_done" })), null);
  assert.equal(
    parseContent(
      manifest({
        artifacts: [
          {
            kind: "database_row",
            label: "Unexpected",
            reference: "row-1",
          },
        ],
      }),
    ),
    null,
  );
  assert.equal(
    parseContent(
      manifest({
        verification: [{ label: "check", status: "maybe" }],
      }),
    ),
    null,
  );
});

test("rejects invalid disposition combinations", () => {
  assert.equal(
    parseContent(manifest({ disposition: "completed", artifacts: [] })),
    null,
  );
  assert.equal(
    parseContent(manifest({ disposition: "no_artifact", artifacts: [{}] })),
    null,
  );
  assert.equal(
    parseContent(
      manifest({ disposition: "blocked", artifacts: [], blocker: undefined }),
    ),
    null,
  );

  const blocked = parseContent(
    manifest({
      disposition: "blocked",
      artifacts: [],
      blocker: "Maintainer decision required.",
    }),
  );
  assert.equal(blocked?.blocker, "Maintainer decision required.");
  assert.equal(getJobResultFeedHeadline(blocked.disposition), "Job blocked");
});

test("rejects control characters in single-line artifact fields", () => {
  assert.equal(
    parseContent(
      manifest({
        artifacts: [
          {
            kind: "other",
            label: "Artifact",
            reference: "run-1\nprivate-note",
          },
        ],
      }),
    ),
    null,
  );
});

test("rejects unsafe URL credentials and upward file traversal", () => {
  for (const artifact of [
    {
      kind: "link",
      label: "Credential-bearing URL",
      reference: "https://user:secret@example.com/report",
    },
    {
      kind: "link",
      label: "Hostless URL",
      reference: "https://",
    },
    {
      kind: "other",
      label: "Credential-bearing generic artifact",
      reference: "https://user:secret@example.com/report",
    },
    {
      kind: "file",
      label: "Private file",
      reference: "../private.txt",
    },
    {
      kind: "file",
      label: "Windows rooted private file",
      reference: String.raw`\Users\Brad\secret.txt`,
    },
    {
      kind: "file",
      label: "Windows UNC private file",
      reference: String.raw`\\server\share\secret.txt`,
    },
  ]) {
    assert.equal(parseContent(manifest({ artifacts: [artifact] })), null);
  }
});

test("rejects absolute local paths for every raw reference kind", () => {
  for (const kind of ["branch", "canvas", "workflow_output", "other"]) {
    for (const reference of [
      "/Users/Brad/private.txt",
      "~/private.txt",
      String.raw`\Users\Brad\private.txt`,
      String.raw`\\server\share\private.txt`,
      String.raw`C:\Users\Brad\private.txt`,
      "safe/../../private.txt",
    ]) {
      assert.equal(
        parseContent(
          manifest({
            artifacts: [{ kind, label: "Unsafe local reference", reference }],
          }),
        ),
        null,
        `${kind} reference ${reference} should be rejected`,
      );
    }
  }
});

test("requires one reply tag matching the payload job request", () => {
  const tags = [["e", JOB_REQUEST, "", "reply"]];
  assert.equal(getJobResultRequestId(tags), JOB_REQUEST);
  assert.equal(
    parseContent(manifest(), getJobResultRequestId(tags))?.jobRequest,
    JOB_REQUEST,
  );

  const differentRequest = "b".repeat(64);
  assert.equal(parseContent(manifest(), differentRequest), null);
  assert.equal(getJobResultRequestId([]), null);
  assert.equal(
    getJobResultRequestId([
      ["e", JOB_REQUEST, "", "reply"],
      ["e", differentRequest, "", "reply"],
    ]),
    null,
  );
});

test("rejects content and fields that exceed byte limits before trimming", () => {
  assert.equal(
    parseContent(
      manifest({
        futureField: "x".repeat(64 * 1024),
      }),
    ),
    null,
  );
  assert.equal(
    parseContent(manifest({ outcome: `ready${" ".repeat(8 * 1024)}` })),
    null,
  );
});
