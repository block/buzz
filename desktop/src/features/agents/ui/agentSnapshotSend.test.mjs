import assert from "node:assert/strict";
import test from "node:test";

// Tests for the snapshot send controller helpers and dialog behavior.

import { isSendableDestination } from "./useSnapshotSendController.ts";

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

test("memory_gate_step_shows_plaintext_core_memory_label", () => {
  const el = MemoryGateStep({
    destination: makeDestination(),
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /plaintext\s+core\s+memory/i, `got: ${text}`);
});

test("memory_gate_step_shows_plaintext_all_memory_label", () => {
  const el = MemoryGateStep({
    destination: makeDestination(),
    memoryLevel: "everything",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /plaintext\s+all\s+memory/i, `got: ${text}`);
});

test("memory_gate_step_names_channel_destination", () => {
  const el = MemoryGateStep({
    destination: makeDestination({ channelType: "stream", name: "team-alpha" }),
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /#team-alpha/i, `got: ${text}`);
});

test("memory_gate_step_names_dm_destination", () => {
  const el = MemoryGateStep({
    destination: makeDestination({ channelType: "dm", name: "Alice" }),
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /the DM with Alice/i, `got: ${text}`);
});

test("memory_gate_step_discloses_media_link_access", () => {
  const el = MemoryGateStep({
    destination: makeDestination(),
    memoryLevel: "core",
  });
  const text = collectText(el).join(" ");
  assert.match(text, /media link/i, `got: ${text}`);
});
