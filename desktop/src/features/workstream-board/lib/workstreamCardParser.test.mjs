import assert from "node:assert/strict";
import test from "node:test";

import { parseWorkstreamCard } from "./workstreamCardParser.ts";

// Helper: build a fenced card body from a JSON-serializable payload (or raw
// string, to construct intentionally-invalid-JSON fixtures).
function withCardFence(prose, rawPayload) {
  const body =
    typeof rawPayload === "string" ? rawPayload : JSON.stringify(rawPayload);
  return `${prose}\n\n\`\`\`buzz-workstream-card\n${body}\n\`\`\``;
}

const VALID_PAYLOAD = {
  version: 1,
  synopsis: "Implementing the canvas card slice.",
  orchestrator: { pubkey: "loganj-pubkey", name: "Logan" },
  assignees: [
    { pubkey: "alice-pubkey", name: "Alice" },
    { pubkey: "bob-pubkey", name: "Bob" },
  ],
};

// ── Happy path ────────────────────────────────────────────────────────────────

test("parses a valid v1 card with explicit optional arrays", () => {
  const result = parseWorkstreamCard(
    withCardFence("Status update:", {
      ...VALID_PAYLOAD,
      pullRequests: ["https://github.com/block/buzz/pull/1"],
      waitingOn: ["review"],
    }),
  );

  assert.equal(result.ok, true);
  assert.deepEqual(result.card, {
    version: 1,
    synopsis: VALID_PAYLOAD.synopsis,
    orchestrator: VALID_PAYLOAD.orchestrator,
    assignees: VALID_PAYLOAD.assignees,
    pullRequests: ["https://github.com/block/buzz/pull/1"],
    waitingOn: ["review"],
  });
});

test("defaults assignees/pullRequests/waitingOn to empty arrays when omitted", () => {
  const result = parseWorkstreamCard(
    withCardFence("Status update:", VALID_PAYLOAD),
  );

  assert.equal(result.ok, true);
  assert.deepEqual(result.card.assignees, VALID_PAYLOAD.assignees);
  assert.deepEqual(result.card.pullRequests, []);
  assert.deepEqual(result.card.waitingOn, []);
});

test("ignores prose surrounding the fence", () => {
  const content = [
    "# Workstream",
    "",
    "Some human-authored notes above the card.",
    "",
    "```buzz-workstream-card",
    JSON.stringify(VALID_PAYLOAD),
    "```",
    "",
    "Notes below the card too.",
  ].join("\n");

  const result = parseWorkstreamCard(content);
  assert.equal(result.ok, true);
  assert.equal(result.card.synopsis, VALID_PAYLOAD.synopsis);
});

// ── Missing block ─────────────────────────────────────────────────────────────

test("returns not-found for null content", () => {
  assert.deepEqual(parseWorkstreamCard(null), {
    ok: false,
    reason: "not-found",
  });
});

test("returns not-found for empty content", () => {
  assert.deepEqual(parseWorkstreamCard(""), { ok: false, reason: "not-found" });
});

test("returns not-found when canvas has prose but no fence", () => {
  assert.deepEqual(parseWorkstreamCard("Just some notes, no card here."), {
    ok: false,
    reason: "not-found",
  });
});

// ── Invalid JSON ──────────────────────────────────────────────────────────────

test("returns invalid-json for malformed JSON inside the fence", () => {
  const content = withCardFence("Status:", "{not valid json");
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-json",
  });
});

// ── Duplicate blocks ──────────────────────────────────────────────────────────

test("returns duplicate-block when the canvas has two card fences", () => {
  const content = [
    withCardFence("First:", VALID_PAYLOAD),
    "",
    withCardFence("Second:", VALID_PAYLOAD),
  ].join("\n");

  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "duplicate-block",
  });
});

// ── Unknown version ───────────────────────────────────────────────────────────

test("returns unknown-version for version 2", () => {
  const content = withCardFence("Status:", { ...VALID_PAYLOAD, version: 2 });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "unknown-version",
  });
});

test("returns unknown-version when version is missing", () => {
  const { version: _version, ...withoutVersion } = VALID_PAYLOAD;
  const content = withCardFence("Status:", withoutVersion);
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "unknown-version",
  });
});

// ── Missing / invalid required fields ────────────────────────────────────────

test("returns invalid-fields when synopsis is missing", () => {
  const { synopsis: _synopsis, ...withoutSynopsis } = VALID_PAYLOAD;
  const content = withCardFence("Status:", withoutSynopsis);
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when orchestrator is missing its identity fields", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    orchestrator: { name: "Logan" },
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when assignees is not an array", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    assignees: "alice",
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when assignees contains an invalid identity", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    assignees: [{ pubkey: "alice-pubkey", name: "Alice" }, "bob"],
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when pullRequests is not an array", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    pullRequests: "pr-1",
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when waitingOn is not an array", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    waitingOn: "review",
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("rejects the former string identity schema for orchestrator", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    orchestrator: "loganj",
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("rejects the former string identity schema for assignees", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    assignees: ["alice"],
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when the payload is a JSON array, not an object", () => {
  const content = withCardFence("Status:", [VALID_PAYLOAD]);
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when synopsis spans multiple lines", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    synopsis: "line one\nline two",
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});

test("returns invalid-fields when optional arrays are explicitly null", () => {
  const content = withCardFence("Status:", {
    ...VALID_PAYLOAD,
    pullRequests: null,
  });
  assert.deepEqual(parseWorkstreamCard(content), {
    ok: false,
    reason: "invalid-fields",
  });
});
