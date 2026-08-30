import assert from "node:assert/strict";
import test from "node:test";

import { parseWorkReport, reduceWorkReports } from "./workReport.ts";

const CHANNEL = "4d1413c0-24f3-4df6-9838-9de4373feb1e";
const ROOT = "a".repeat(64);

function reportEvent({
  id = "b".repeat(64),
  createdAt = 100,
  status = "completed",
  outcome = "Shipped the result card.",
  tags = [],
} = {}) {
  return {
    id,
    pubkey: "c".repeat(64),
    created_at: createdAt,
    kind: 40009,
    tags: [
      ["h", CHANNEL],
      ["e", ROOT, "", "root"],
      ["t", "work-report"],
      ["status", status],
      ...tags,
    ],
    content: JSON.stringify({
      status,
      outcome,
      deliverables: ["https://example.com/pr/1"],
      decisions: ["Keep the transcript as evidence."],
      verification: ["CI passed."],
      risks: [],
      next_actions: ["Maintainer: review."],
    }),
    sig: "d".repeat(128),
  };
}

test("parses a channel and thread-rooted work report", () => {
  const parsed = parseWorkReport(reportEvent(), CHANNEL, ROOT);
  assert.equal(parsed?.status, "completed");
  assert.equal(parsed?.outcome, "Shipped the result card.");
  assert.deepEqual(parsed?.verification, ["CI passed."]);
});

test("rejects reports whose envelope and body statuses disagree", () => {
  const event = reportEvent();
  event.tags = event.tags.map((tag) =>
    tag[0] === "status" ? ["status", "blocked"] : tag,
  );
  assert.equal(parseWorkReport(event, CHANNEL, ROOT), null);
});

test("rejects reports scoped to another thread", () => {
  assert.equal(parseWorkReport(reportEvent(), CHANNEL, "e".repeat(64)), null);
});

test("reduces valid reports by created_at then event id", () => {
  const older = reportEvent({ id: "1".repeat(64), createdAt: 100 });
  const lowerId = reportEvent({ id: "2".repeat(64), createdAt: 200 });
  const head = reportEvent({
    id: "3".repeat(64),
    createdAt: 200,
    status: "in_review",
    outcome: "Ready for review.",
    tags: [["prior", lowerId.id]],
  });
  const reduced = reduceWorkReports([head, older, lowerId], CHANNEL, ROOT);
  assert.equal(reduced?.eventId, head.id);
  assert.equal(reduced?.status, "in_review");
  assert.equal(reduced?.prior, lowerId.id);
});

test("ignores malformed newer events when choosing the head", () => {
  const valid = reportEvent({ id: "4".repeat(64), createdAt: 100 });
  const malformed = reportEvent({ id: "5".repeat(64), createdAt: 300 });
  malformed.content = "not-json";
  assert.equal(
    reduceWorkReports([valid, malformed], CHANNEL, ROOT)?.eventId,
    valid.id,
  );
});
