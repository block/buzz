import assert from "node:assert/strict";
import test from "node:test";

import { createPendingProfileSave } from "./pendingProfileSave.ts";

function harness(overrides = {}) {
  const calls = { saves: [] };
  let stored = overrides.stored ?? null;
  const deps = {
    read: () => stored,
    write: (value) => {
      stored = value;
    },
    remove: () => {
      stored = null;
    },
    saveProfile: async (input) => {
      calls.saves.push(input);
      if (overrides.saveError) throw overrides.saveError;
      return { pubkey: "abc", displayName: input.displayName ?? null };
    },
    getActivePubkey: async () => {
      if (overrides.pubkeyError) throw overrides.pubkeyError;
      // `??` would swallow an explicitly-null activePubkey, which is a case
      // under test — distinguish "not provided" from "provided as null".
      return "activePubkey" in overrides ? overrides.activePubkey : "abc";
    },
  };
  return {
    calls,
    sync: createPendingProfileSave(deps),
    stored: () => stored,
  };
}

const PENDING = { pubkey: "abc", displayName: "Barbee", avatarUrl: "" };

test("flush with nothing pending is a no-op", async () => {
  const h = harness();
  assert.equal(await h.sync.flush(), "empty");
  assert.equal(h.calls.saves.length, 0);
});

test("flush writes the parked profile and clears it", async () => {
  const h = harness();
  h.sync.save(PENDING);
  assert.equal(await h.sync.flush(), "saved");
  assert.deepEqual(h.calls.saves, [{ displayName: "Barbee" }]);
  assert.equal(h.stored(), null);
});

test("flush keeps the value parked while membership is still denied", async () => {
  // The case this module exists for: the user typed a name before joining.
  const h = harness({ saveError: new Error("relay returned 404 Not Found") });
  h.sync.save(PENDING);
  assert.equal(await h.sync.flush(), "deferred");
  assert.notEqual(h.stored(), null, "must survive for the next attempt");
  assert.deepEqual(h.sync.peek(), PENDING);
});

test("flush discards a value belonging to a different identity", async () => {
  const h = harness({ activePubkey: "def" });
  h.sync.save(PENDING);
  assert.equal(await h.sync.flush(), "discarded");
  assert.equal(h.calls.saves.length, 0, "must not write under another key");
  assert.equal(h.stored(), null);
});

test("identity comparison ignores case", async () => {
  const h = harness({ activePubkey: "ABC" });
  h.sync.save(PENDING);
  assert.equal(await h.sync.flush(), "saved");
});

test("flush defers when no identity is resolvable yet", async () => {
  const h = harness({ activePubkey: null });
  h.sync.save(PENDING);
  assert.equal(await h.sync.flush(), "deferred");
  assert.notEqual(h.stored(), null);

  const thrown = harness({ pubkeyError: new Error("keyring locked") });
  thrown.sync.save(PENDING);
  assert.equal(await thrown.sync.flush(), "deferred");
  assert.notEqual(thrown.stored(), null);
});

test("flush discards on a non-membership save failure", async () => {
  // Retrying forever would wedge every later boot.
  const h = harness({ saveError: new Error("relay returned 400: malformed") });
  h.sync.save(PENDING);
  assert.equal(await h.sync.flush(), "discarded");
  assert.equal(h.stored(), null);
});

test("saving an empty profile clears rather than parks", async () => {
  const h = harness();
  h.sync.save({ pubkey: "abc", displayName: "Barbee", avatarUrl: "" });
  h.sync.save({ pubkey: "abc", displayName: "   ", avatarUrl: "  " });
  assert.equal(h.stored(), null);
  assert.equal(await h.sync.flush(), "empty");
});

test("avatar-only pending values are replayed", async () => {
  const h = harness();
  h.sync.save({ pubkey: "abc", displayName: "", avatarUrl: "https://x/a.png" });
  assert.equal(await h.sync.flush(), "saved");
  assert.deepEqual(h.calls.saves, [{ avatarUrl: "https://x/a.png" }]);
});

test("both fields are replayed together", async () => {
  const h = harness();
  h.sync.save({
    pubkey: "abc",
    displayName: " Barbee ",
    avatarUrl: " https://x/a.png ",
  });
  assert.equal(await h.sync.flush(), "saved");
  assert.deepEqual(h.calls.saves, [
    { displayName: "Barbee", avatarUrl: "https://x/a.png" },
  ]);
});

test("malformed or foreign storage contents are ignored", async () => {
  for (const stored of [
    "not json",
    "null",
    '{"pubkey":"abc"}',
    '{"displayName":"x","avatarUrl":""}',
    '{"pubkey":"","displayName":"x","avatarUrl":""}',
    "[1,2,3]",
  ]) {
    const h = harness({ stored });
    assert.equal(h.sync.peek(), null, `should reject: ${stored}`);
    assert.equal(await h.sync.flush(), "empty");
  }
});

test("storage failures never propagate", async () => {
  const sync = createPendingProfileSave({
    read: () => {
      throw new Error("storage disabled");
    },
    write: () => {
      throw new Error("quota exceeded");
    },
    remove: () => {
      throw new Error("storage disabled");
    },
    saveProfile: async () => ({ pubkey: "abc" }),
    getActivePubkey: async () => "abc",
  });
  assert.doesNotThrow(() => sync.save(PENDING));
  assert.equal(sync.peek(), null);
  assert.equal(await sync.flush(), "empty");
});
