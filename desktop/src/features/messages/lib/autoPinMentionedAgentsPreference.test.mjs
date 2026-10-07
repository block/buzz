import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};

const preference = await import("./autoPinMentionedAgentsPreference.ts");

test("defaults missing and invalid values to keeping mentioned agents pinned", () => {
  assert.equal(preference.parseKeepMentionedAgentsPinned(null), true);
  assert.equal(preference.parseKeepMentionedAgentsPinned("invalid"), true);
  assert.equal(preference.parseKeepMentionedAgentsPinned("true"), true);
  assert.equal(preference.parseKeepMentionedAgentsPinned("false"), false);
});

test("persists changes to the post-mention pinning preference", () => {
  preference.setKeepMentionedAgentsPinned(false);
  assert.equal(preference.getKeepMentionedAgentsPinned(), false);
  assert.equal(
    values.get(preference.KEEP_MENTIONED_AGENTS_PINNED_STORAGE_KEY),
    "false",
  );

  preference.setKeepMentionedAgentsPinned(true);
  assert.equal(preference.getKeepMentionedAgentsPinned(), true);
  assert.equal(
    values.get(preference.KEEP_MENTIONED_AGENTS_PINNED_STORAGE_KEY),
    "true",
  );
});
