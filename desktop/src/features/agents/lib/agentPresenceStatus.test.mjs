import assert from "node:assert/strict";
import test from "node:test";

import { mergeAgentPresenceStatus } from "./agentPresenceStatus.ts";

test("mergeAgentPresenceStatus — live presence overrides the kind:10100 offline fallback", () => {
  // The bug this fixes: `agents_from_events` defaults every relay-discovered
  // agent to "offline" because nothing writes `status` into kind:10100, so a
  // running agent was reported offline.
  const map = mergeAgentPresenceStatus([{ pubkey: "a", status: "offline" }], {
    a: "online",
  });
  assert.equal(map.a, "online");
});

test("mergeAgentPresenceStatus — presence reporting away is preserved", () => {
  const map = mergeAgentPresenceStatus([{ pubkey: "a", status: "offline" }], {
    a: "away",
  });
  assert.equal(map.a, "away");
});

test("mergeAgentPresenceStatus — agents with no presence entry keep their own status", () => {
  // Locally-managed agents derive status from the live process handle, which
  // the relay cannot contradict for the machine running them.
  const map = mergeAgentPresenceStatus([{ pubkey: "a", status: "online" }], {});
  assert.equal(map.a, "online");
});

test("mergeAgentPresenceStatus — presence for an unrelated pubkey does not leak in", () => {
  const map = mergeAgentPresenceStatus([{ pubkey: "a", status: "offline" }], {
    b: "online",
  });
  assert.deepEqual(map, { a: "offline" });
});

test("mergeAgentPresenceStatus — a presence lookup that has not loaded yet is a no-op", () => {
  const map = mergeAgentPresenceStatus(
    [
      { pubkey: "a", status: "online" },
      { pubkey: "b", status: "offline" },
    ],
    undefined,
  );
  assert.deepEqual(map, { a: "online", b: "offline" });
});

test("mergeAgentPresenceStatus — presence can also move an agent to offline", () => {
  // A managed agent whose process died still reports "online" from a stale
  // handle; relay presence expiring is what corrects it.
  const map = mergeAgentPresenceStatus([{ pubkey: "a", status: "online" }], {
    a: "offline",
  });
  assert.equal(map.a, "offline");
});
