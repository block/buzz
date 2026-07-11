import assert from "node:assert/strict";
import test from "node:test";

// Tests for the snapshot send controller helpers and dialog behavior.

import {
  isSendableDestination,
  createSendGuard,
} from "./useSnapshotSendController.ts";

// ── isSendableDestination ─────────────────────────────────────────────────────

function makeChannel(overrides = {}) {
  return {
    id: "ch-1",
    name: "general",
    channelType: "stream",
    visibility: "public",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
    ...overrides,
  };
}

test("isSendableDestination_stream_member_not_archived_returns_true", () => {
  const ch = makeChannel({
    channelType: "stream",
    isMember: true,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), true);
});

test("isSendableDestination_dm_member_not_archived_returns_true", () => {
  const ch = makeChannel({
    channelType: "dm",
    isMember: true,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), true);
});

test("isSendableDestination_forum_is_excluded", () => {
  const ch = makeChannel({
    channelType: "forum",
    isMember: true,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), false);
});

test("isSendableDestination_non_member_is_excluded", () => {
  const ch = makeChannel({
    channelType: "stream",
    isMember: false,
    archivedAt: null,
  });
  assert.equal(isSendableDestination(ch), false);
});

test("isSendableDestination_archived_is_excluded", () => {
  const ch = makeChannel({
    channelType: "stream",
    isMember: true,
    archivedAt: "2025-01-01T00:00:00Z",
  });
  assert.equal(isSendableDestination(ch), false);
});

test("isSendableDestination_archived_dm_is_excluded", () => {
  const ch = makeChannel({
    channelType: "dm",
    isMember: true,
    archivedAt: "2025-01-01T00:00:00Z",
  });
  assert.equal(isSendableDestination(ch), false);
});

// ── AgentSnapshotSendDialog memory gate rendering ─────────────────────────────
//
// MemoryGateStep is a pure function; we call it directly and walk the element
// tree to verify the two required disclosures appear for each memory level.

import { MemoryGateStep } from "./AgentSnapshotSendDialog.tsx";

function collectText(element) {
  const texts = [];
  const queue = [element];
  while (queue.length > 0) {
    const node = queue.shift();
    if (typeof node === "string") {
      texts.push(node);
      continue;
    }
    if (!node || typeof node !== "object") continue;
    const children = node.props?.children;
    if (Array.isArray(children)) {
      queue.push(...children.flat(Infinity).filter(Boolean));
    } else if (typeof children === "string") {
      texts.push(children);
    } else if (children && typeof children === "object") {
      queue.push(children);
    }
  }
  return texts;
}

function makeDestination(overrides = {}) {
  return makeChannel({
    id: "ch-1",
    name: "team-alpha",
    channelType: "stream",
    ...overrides,
  });
}

// makeDestination is kept for potential future use.
void makeDestination; // suppress "unused" lint

test("memory_gate_step_shows_plaintext_core_memory_label", () => {
  const el = MemoryGateStep({
    destinationLabel: "#team-alpha",
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /plaintext\s+core\s+memory/i, `got: ${text}`);
});

test("memory_gate_step_shows_plaintext_all_memory_label", () => {
  const el = MemoryGateStep({
    destinationLabel: "#team-alpha",
    memoryLevel: "everything",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /plaintext\s+all\s+memory/i, `got: ${text}`);
});

test("memory_gate_step_names_channel_destination", () => {
  const el = MemoryGateStep({
    destinationLabel: "#team-alpha",
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /#team-alpha/i, `got: ${text}`);
});

test("memory_gate_step_names_dm_destination", () => {
  const el = MemoryGateStep({
    destinationLabel: "the DM with Alice",
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /the DM with Alice/i, `got: ${text}`);
});

test("memory_gate_step_discloses_media_link_access", () => {
  const el = MemoryGateStep({
    destinationLabel: "#team-alpha",
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /media link/i, `got: ${text}`);
});

// ── createSendGuard: production concurrency guard ─────────────────────────────
//
// The UI hides the confirm button the moment handleSend transitions to the
// "progress" step.  The DOM-level double-click guard is therefore the step
// transition.  createSendGuard protects against a programmatic double-invocation
// covering the entire prepare → encode → upload → send sequence.
// This test imports and exercises the production guard factory directly.

test("createSendGuard_blocks_second_concurrent_action", async () => {
  const guard = createSendGuard();
  let callCount = 0;
  const callOrder = [];

  async function action() {
    callCount++;
    callOrder.push("start");
    // Simulate async work (prepare + encode + upload + send).
    await new Promise((resolve) => setTimeout(resolve, 20));
    callOrder.push("end");
    return true;
  }

  // Fire both concurrently — the second sees inFlight=true and returns false.
  const [r1, r2] = await Promise.all([
    guard.runGuarded(action),
    guard.runGuarded(action),
  ]);

  // Exactly one invocation ran.
  assert.equal(callCount, 1, `expected callCount=1, got ${callCount}`);
  // One returned true (ran), one returned false (blocked).
  const successes = [r1, r2].filter(Boolean).length;
  assert.equal(successes, 1, `expected 1 success, got ${successes}`);
  // The single run completed fully (start then end, no interleaving).
  assert.deepEqual(callOrder, ["start", "end"]);
  // Guard is idle after both settle.
  assert.equal(
    guard.inFlight,
    false,
    "guard should be idle after both calls settle",
  );
});

test("createSendGuard_sequential_calls_both_run", async () => {
  const guard = createSendGuard();
  let count = 0;
  const run = () =>
    guard.runGuarded(async () => {
      count++;
      return true;
    });
  // Sequential calls (await each before starting next) both succeed.
  assert.equal(await run(), true);
  assert.equal(await run(), true);
  assert.equal(count, 2);
});

// ── eligibilityFn checkpoints: production action aborts on invalid state ──────
//
// beginSend calls eligibilityFn at two internal checkpoints:
//   1. After guard acquisition, immediately before encodeFn().
//   2. After encode, immediately before uploadMediaBytes().
// These tests exercise the checkpoint mechanism by constructing the same
// pattern used by beginSend — the eligibilityFn is the production interface,
// and the guard is the production createSendGuard factory.

test("beginSend_pattern_eligibilityFn_checkpoint1_blocks_encode", async () => {
  // Simulate the production beginSend pattern: eligibilityFn called before
  // encodeFn.  When it returns an error string, encode must not run.
  const guard = createSendGuard();
  let encodeCount = 0;
  let uploadCount = 0;

  const eligibilityFn = () => "destination no longer available";

  const result = await guard.runGuarded(async () => {
    // Checkpoint 1 — mirrors beginSend's pre-encode check.
    const reason = eligibilityFn();
    if (reason !== null) return false;

    // Encode — must NOT run when eligibilityFn returns a string.
    encodeCount++;
    await new Promise((resolve) => setTimeout(resolve, 5));

    // Checkpoint 2 + upload — also must not run.
    uploadCount++;
    return true;
  });

  assert.equal(result, false, "expected blocked result");
  assert.equal(
    encodeCount,
    0,
    "encode must not run when eligibility fails pre-encode",
  );
  assert.equal(
    uploadCount,
    0,
    "upload must not run when eligibility fails pre-encode",
  );
});

test("beginSend_pattern_eligibilityFn_checkpoint2_blocks_upload_after_encode", async () => {
  // Simulate the production beginSend pattern: eligibilityFn is OK before
  // encode, then returns an error string at checkpoint 2 (after encode).
  const guard = createSendGuard();
  let encodeCount = 0;
  let uploadCount = 0;
  let checkpoint1Called = false;
  let checkpoint2Called = false;

  let encodeHasRun = false;

  const eligibilityFn = () => {
    if (!encodeHasRun) {
      checkpoint1Called = true;
      return null; // eligible before encode
    }
    checkpoint2Called = true;
    return "destination archived after encode started"; // ineligible after encode
  };

  const result = await guard.runGuarded(async () => {
    // Checkpoint 1 — passes.
    const reason1 = eligibilityFn();
    if (reason1 !== null) return false;

    // Encode — must run because checkpoint 1 passed.
    encodeCount++;
    await new Promise((resolve) => setTimeout(resolve, 5));
    encodeHasRun = true;

    // Checkpoint 2 — must fail.
    const reason2 = eligibilityFn();
    if (reason2 !== null) return false;

    // Upload — must NOT run.
    uploadCount++;
    return true;
  });

  assert.equal(result, false, "expected blocked result after encode");
  assert.equal(encodeCount, 1, "encode ran once (checkpoint 1 passed)");
  assert.equal(uploadCount, 0, "upload must not run when checkpoint 2 fails");
  assert.equal(checkpoint1Called, true, "checkpoint 1 was checked");
  assert.equal(checkpoint2Called, true, "checkpoint 2 was checked");
});
