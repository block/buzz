import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
const attributes = new Map();

globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};
globalThis.document = {
  documentElement: {
    setAttribute: (name, value) => attributes.set(name, value),
  },
};

const preference = await import("./conversationFontSizePreference.ts");

test("defaults invalid and missing conversation font sizes to default", () => {
  assert.equal(preference.parseConversationFontSize(null), "default");
  assert.equal(preference.parseConversationFontSize("medium"), "default");
  assert.equal(preference.parseConversationFontSize("smaller"), "smaller");
  assert.equal(preference.parseConversationFontSize("default"), "default");
  assert.equal(preference.parseConversationFontSize("larger"), "larger");
});

test("persists and applies the selected conversation font size", () => {
  preference.setConversationFontSize("smaller");
  assert.equal(preference.getConversationFontSize(), "smaller");
  assert.equal(
    values.get(preference.CONVERSATION_FONT_SIZE_STORAGE_KEY),
    "smaller",
  );
  assert.equal(attributes.get("data-conversation-font-size"), "smaller");
});

test("previews a font size without changing the saved preference", () => {
  preference.setConversationFontSize("smaller");
  preference.previewConversationFontSize("larger");
  assert.equal(preference.getConversationFontSize(), "smaller");
  assert.equal(
    values.get(preference.CONVERSATION_FONT_SIZE_STORAGE_KEY),
    "smaller",
  );
  assert.equal(attributes.get("data-conversation-font-size"), "larger");

  preference.previewConversationFontSize(null);
  assert.equal(attributes.get("data-conversation-font-size"), "smaller");
});

test("initializes from the stored conversation font size", () => {
  values.set(preference.CONVERSATION_FONT_SIZE_STORAGE_KEY, "larger");
  preference.initializeConversationFontSizePreference();
  assert.equal(preference.getConversationFontSize(), "larger");
  assert.equal(attributes.get("data-conversation-font-size"), "larger");
});
