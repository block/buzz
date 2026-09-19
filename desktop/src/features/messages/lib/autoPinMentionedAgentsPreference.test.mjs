import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};

const preference = await import("./autoPinMentionedAgentsPreference.ts");
const persistentAudience = await import("./persistentAgentAudience.ts");

test("defaults missing and invalid values to one-time agent mentions", () => {
  assert.equal(preference.parseKeepMentionedAgentsPinned(null), false);
  assert.equal(preference.parseKeepMentionedAgentsPinned("invalid"), false);
  assert.equal(preference.parseKeepMentionedAgentsPinned("true"), true);
  assert.equal(preference.parseKeepMentionedAgentsPinned("false"), false);
});

test("persists changes to the post-mention pinning preference", () => {
  preference.setKeepMentionedAgentsPinned(true);
  assert.equal(preference.getKeepMentionedAgentsPinned(), true);
  assert.equal(
    values.get(preference.KEEP_MENTIONED_AGENTS_PINNED_STORAGE_KEY),
    "true",
  );

  preference.setKeepMentionedAgentsPinned(false);
  assert.equal(preference.getKeepMentionedAgentsPinned(), false);
  assert.equal(
    values.get(preference.KEEP_MENTIONED_AGENTS_PINNED_STORAGE_KEY),
    "false",
  );
});

test("turning off automatic mentions clears active conversation audiences", () => {
  const scope = `${"1".repeat(64)}:channel-a:channel`;
  persistentAudience.setPersistentAgentAudience(scope, ["a".repeat(64)]);
  preference.setKeepMentionedAgentsPinned(true);

  preference.setKeepMentionedAgentsPinned(false);

  assert.deepEqual(
    persistentAudience.getPersistentAgentAudienceSnapshot().audiences,
    {},
  );
});
