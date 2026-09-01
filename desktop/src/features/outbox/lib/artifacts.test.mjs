import assert from "node:assert/strict";
import test from "node:test";

import { buildOutboxArtifacts } from "./artifacts.ts";

const AGENT = "a".repeat(64);
const HUMAN = "b".repeat(64);

function event(overrides = {}) {
  return {
    id: "c".repeat(64),
    pubkey: AGENT,
    created_at: 1_700_000_000,
    kind: 9,
    tags: [
      ["h", "channel-1"],
      ["buzz-outbox", "1"],
      [
        "imeta",
        "url https://relay.example/media/report.pdf",
        "m application/pdf",
        `x ${"d".repeat(64)}`,
        "size 2048",
        "filename QUARTERLY_REPORT.pdf",
      ],
    ],
    content:
      "The review package is complete.\n\n[QUARTERLY_REPORT.pdf](https://relay.example/media/report.pdf)",
    sig: "e".repeat(128),
    ...overrides,
  };
}

test("projects agent attachments into newest-first outbox rows", () => {
  const older = event({ id: "1".repeat(64), created_at: 100 });
  const newer = event({
    id: "2".repeat(64),
    created_at: 200,
    tags: [
      ["h", "channel-2"],
      ["buzz-outbox", "1"],
      [
        "imeta",
        "url https://relay.example/media/mockup.png",
        "m image/png",
        `x ${"f".repeat(64)}`,
        "size 4096",
        "filename MOCKUP.png",
      ],
    ],
  });

  const artifacts = buildOutboxArtifacts([older, newer], new Set([AGENT]));

  assert.deepEqual(
    artifacts.map((artifact) => artifact.filename),
    ["MOCKUP.png", "QUARTERLY_REPORT.pdf"],
  );
  assert.equal(artifacts[0].kind, "image");
  assert.equal(artifacts[1].kind, "document");
  assert.equal(artifacts[1].channelId, "channel-1");
  assert.equal(artifacts[1].sourceSummary, "The review package is complete.");
});

test("ignores human attachments, unmarked files, and messages without files", () => {
  const humanAttachment = event({ pubkey: HUMAN });
  const agentMessage = event({ tags: [["h", "channel-1"]] });
  const unmarkedAttachment = event({
    id: "4".repeat(64),
    tags: event().tags.filter((tag) => tag[0] !== "buzz-outbox"),
  });

  assert.deepEqual(
    buildOutboxArtifacts(
      [humanAttachment, agentMessage, unmarkedAttachment],
      new Set([AGENT]),
    ),
    [],
  );
});

test("falls back safely when optional imeta metadata is absent", () => {
  const artifact = buildOutboxArtifacts(
    [
      event({
        tags: [
          ["h", "channel-1"],
          ["buzz-outbox", "1"],
          ["imeta", "url https://relay.example/media/notes.txt"],
        ],
      }),
    ],
    new Set([AGENT.toUpperCase()]),
  )[0];

  assert.equal(artifact.filename, "notes.txt");
  assert.equal(artifact.mimeType, "application/octet-stream");
  assert.equal(artifact.size, undefined);
});
