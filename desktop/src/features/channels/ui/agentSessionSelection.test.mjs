import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveAgentSessionReturnTarget,
  resolveSelectedAgentSession,
} from "./agentSessionSelection.ts";

test("uses a name-only Nostr profile for a fallback agent session", () => {
  const pubkey = "a".repeat(64);

  assert.equal(
    resolveSelectedAgentSession({
      agentSessionAgents: [],
      openAgentSessionPubkey: pubkey,
      profilePanelPubkey: pubkey,
      profiles: {
        [pubkey]: {
          avatarUrl: null,
          displayName: null,
          name: "Bumble",
          pubkey,
        },
      },
    })?.name,
    "Bumble",
  );
});

test("returns the open thread when activity opens over a thread", () => {
  assert.deepEqual(
    resolveAgentSessionReturnTarget({
      openThreadHeadId: "head-1",
      profilePanelPubkey: null,
    }),
    { kind: "thread", threadHeadId: "head-1" },
  );
});

test("returns the profile when activity opens over the profile panel", () => {
  assert.deepEqual(
    resolveAgentSessionReturnTarget({
      openThreadHeadId: null,
      profilePanelPubkey: "abc",
    }),
    { kind: "profile", pubkey: "abc" },
  );
});

test("prefers the thread when both params linger, matching pane priority", () => {
  assert.deepEqual(
    resolveAgentSessionReturnTarget({
      openThreadHeadId: "head-1",
      profilePanelPubkey: "abc",
    }),
    { kind: "thread", threadHeadId: "head-1" },
  );
});

test("returns null when activity opens over no pane", () => {
  assert.equal(
    resolveAgentSessionReturnTarget({
      openThreadHeadId: null,
      profilePanelPubkey: null,
    }),
    null,
  );
});
