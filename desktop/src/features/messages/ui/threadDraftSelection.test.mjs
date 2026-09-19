import assert from "node:assert/strict";
import test from "node:test";
import { resolveThreadDraftKey } from "./threadDraftSelection.ts";
import { openDraftEntry, sendDraftEntry } from "./DraftsPanel.tsx";

const root = { channelId: "channel", content: "root draft" };
const legacy = {
  channelId: "channel",
  content: "legacy draft",
  pendingImeta: [{ url: "attachment" }],
};
const store = new Map([
  ["thread:root", root],
  ["thread:response", legacy],
]);
const options = {
  rootId: "root",
  channelId: "channel",
  messageIds: new Set(["response"]),
  loadDraft: (key) => store.get(key),
};

test("legacy draft navigation selects its own key without overwriting the root draft", async () => {
  const entry = { key: "thread:response", draft: legacy };
  for (const navigate of [openDraftEntry, sendDraftEntry]) {
    let navigation;
    await navigate(entry, async (channel, params) => {
      navigation = { channel, params };
    });
    assert.equal(navigation.channel, "channel");
    assert.equal(navigation.params.threadDraftKey, entry.key);
    const key = resolveThreadDraftKey({
      ...options,
      requestedKeys: [navigation.params.threadDraftKey],
    });
    assert.equal(key, entry.key);
    assert.equal(store.get(key), legacy);
    if (navigate === sendDraftEntry)
      assert.equal(navigation.params.autoSend, key);
    assert.equal(store.get("thread:root"), root);
  }
});

test("thread draft resolution refuses other threads and other channels", () => {
  assert.equal(
    resolveThreadDraftKey({ ...options, requestedKeys: ["thread:unrelated"] }),
    "thread:root",
  );
  assert.equal(
    resolveThreadDraftKey({
      ...options,
      channelId: "different",
      requestedKeys: ["thread:response"],
    }),
    "thread:root",
  );
});
