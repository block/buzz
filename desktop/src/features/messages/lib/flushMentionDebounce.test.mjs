import assert from "node:assert/strict";
import test from "node:test";

import { flushMentionDebounce } from "./flushMentionDebounce.ts";

function ref(current) {
  return { current };
}

function candidate(overrides = {}) {
  return {
    kind: "identity",
    displayName: "Beta",
    isAgent: false,
    isMember: true,
    pubkey: "b".repeat(64),
    ...overrides,
  };
}

test("flushMentionDebounce returns the fresh suggestion with its fresh start index", () => {
  const debounceTimerRef = ref(setTimeout(() => {}, 1000));

  const flushed = flushMentionDebounce({
    debounceTimerRef,
    latestValueRef: ref("@Alpha @be"),
    latestCursorRef: ref("@Alpha @be".length),
    searchableNamesLowerRef: ref(["alpha", "beta"]),
    candidates: [
      candidate({ displayName: "Alpha", pubkey: "a".repeat(64) }),
      candidate({ displayName: "Beta", pubkey: "b".repeat(64) }),
    ],
    activePersonaIds: new Set(),
    channelType: "group",
  });

  assert.equal(debounceTimerRef.current, null);
  assert.equal(flushed?.type, "match");
  assert.equal(flushed?.suggestion.displayName, "Beta");
  assert.equal(flushed?.startIndex, 7);
});

test("flushMentionDebounce returns no-match for a fresh query with no matches", () => {
  const flushed = flushMentionDebounce({
    debounceTimerRef: ref(setTimeout(() => {}, 1000)),
    latestValueRef: ref("@Alpha @zzzq"),
    latestCursorRef: ref("@Alpha @zzzq".length),
    searchableNamesLowerRef: ref(["alpha", "beta"]),
    candidates: [candidate()],
    activePersonaIds: new Set(),
    channelType: "group",
  });

  assert.deepEqual(flushed, { type: "no-match" });
});

test("flushMentionDebounce returns null for an empty fresh query", () => {
  const flushed = flushMentionDebounce({
    debounceTimerRef: ref(setTimeout(() => {}, 1000)),
    latestValueRef: ref("@"),
    latestCursorRef: ref("@".length),
    searchableNamesLowerRef: ref(["alpha", "beta"]),
    candidates: [candidate()],
    activePersonaIds: new Set(),
    channelType: "group",
  });

  assert.equal(flushed, null);
});

test("flushMentionDebounce preserves a team expansion selected with Enter", () => {
  const teamMembers = [
    {
      displayName: "Planner",
      kind: "persona",
      personaId: "planner",
    },
    {
      displayName: "Builder",
      kind: "identity",
      personaId: "builder",
      pubkey: "c".repeat(64),
    },
  ];
  const flushed = flushMentionDebounce({
    debounceTimerRef: ref(setTimeout(() => {}, 1000)),
    latestValueRef: ref("Ask @launch"),
    latestCursorRef: ref("Ask @launch".length),
    searchableNamesLowerRef: ref(["launch team"]),
    candidates: [
      candidate({
        kind: "team",
        displayName: "Launch Team",
        isAgent: true,
        isMember: false,
        pubkey: undefined,
        teamId: "launch",
        teamMembers,
      }),
    ],
    activePersonaIds: new Set(),
    channelType: "group",
  });

  assert.equal(flushed?.type, "match");
  assert.equal(flushed?.suggestion.kind, "team");
  assert.deepEqual(flushed?.suggestion.teamMembers, teamMembers);
  assert.equal(flushed?.suggestion.notInChannel, false);
});
