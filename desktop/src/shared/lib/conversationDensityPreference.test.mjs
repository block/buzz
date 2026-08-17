import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
const attributes = new Map();
const windowListeners = new Map();

globalThis.window = {
  addEventListener: (type, listener) => windowListeners.set(type, listener),
};
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};
globalThis.document = {
  documentElement: {
    setAttribute: (name, value) => attributes.set(name, value),
  },
};

const preference = await import("./conversationDensityPreference.ts");

test("defaults invalid and missing conversation densities to comfortable", () => {
  assert.equal(preference.parseConversationDensity(null), "comfortable");
  assert.equal(preference.parseConversationDensity("dense"), "comfortable");
  assert.equal(preference.parseConversationDensity("compact"), "compact");
  assert.equal(
    preference.parseConversationDensity("comfortable"),
    "comfortable",
  );
  assert.equal(preference.parseConversationDensity("spacious"), "spacious");
});

test("persists and applies the selected conversation density", () => {
  preference.setConversationDensity("compact");
  assert.equal(preference.getConversationDensity(), "compact");
  assert.equal(
    values.get(preference.CONVERSATION_DENSITY_STORAGE_KEY),
    "compact",
  );
  assert.equal(attributes.get("data-conversation-density"), "compact");
});

test("previews a density without changing the saved preference", () => {
  preference.setConversationDensity("compact");
  preference.previewConversationDensity("spacious");
  assert.equal(preference.getConversationDensity(), "compact");
  assert.equal(
    values.get(preference.CONVERSATION_DENSITY_STORAGE_KEY),
    "compact",
  );
  assert.equal(attributes.get("data-conversation-density"), "spacious");

  preference.previewConversationDensity(null);
  assert.equal(attributes.get("data-conversation-density"), "compact");
});

test("initializes from the persisted conversation density", () => {
  values.set(preference.CONVERSATION_DENSITY_STORAGE_KEY, "spacious");
  preference.initializeConversationDensityPreference();
  assert.equal(preference.getConversationDensity(), "spacious");
  assert.equal(attributes.get("data-conversation-density"), "spacious");
});

test("applies conversation density changes from another window", () => {
  values.set(preference.CONVERSATION_DENSITY_STORAGE_KEY, "compact");
  windowListeners.get("storage")({
    key: preference.CONVERSATION_DENSITY_STORAGE_KEY,
  });
  assert.equal(preference.getConversationDensity(), "compact");
  assert.equal(attributes.get("data-conversation-density"), "compact");
});

test("returns to comfortable when another window clears storage", () => {
  preference.setConversationDensity("spacious");
  values.clear();
  windowListeners.get("storage")({ key: null });
  assert.equal(preference.getConversationDensity(), "comfortable");
  assert.equal(attributes.get("data-conversation-density"), "comfortable");
});
