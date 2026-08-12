import assert from "node:assert/strict";
import test from "node:test";
import { parseMemoryProposal } from "./memoryProposal.ts";

const id = "a".repeat(64);
const valid = {
  schema: 1,
  status: "proposed",
  kind: "preference",
  scope: "agent",
  targetSlug: "mem/preferences/tone",
  content: "Use plain language.",
  reason: "The owner requested it twice.",
  sourceEventIds: [id],
  evidenceIds: [id],
  confidence: 0.9,
};

test("parses a complete proposal", () =>
  assert.equal(
    parseMemoryProposal("mem/proposals/tone", JSON.stringify(valid))
      ?.targetSlug,
    "mem/preferences/tone",
  ));
test("ignores ordinary memories", () =>
  assert.equal(
    parseMemoryProposal("mem/preferences/tone", JSON.stringify(valid)),
    null,
  ));
test("rejects proposals without evidence-shaped identifiers", () =>
  assert.equal(
    parseMemoryProposal(
      "mem/proposals/tone",
      JSON.stringify({ ...valid, evidenceIds: ["claim"] }),
    ),
    null,
  ));
test("rejects recursive proposal targets", () =>
  assert.equal(
    parseMemoryProposal(
      "mem/proposals/tone",
      JSON.stringify({ ...valid, targetSlug: "mem/proposals/other" }),
    ),
    null,
  ));
