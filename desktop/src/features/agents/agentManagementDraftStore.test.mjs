import assert from "node:assert/strict";
import test from "node:test";

import {
  _testAgentManagementDraftStore,
  enqueueAgentManagementDraft,
  getPendingAgentManagementDraftCount,
  getPendingAgentManagementDrafts,
  initAgentManagementDraftStore,
  peekPendingAgentManagementDraft,
  removeAgentManagementDraft,
  resetAgentManagementDraftStore,
} from "./agentManagementDraftStore.ts";

class MemoryStorage {
  store = new Map();

  get length() {
    return this.store.size;
  }

  getItem(key) {
    return this.store.get(key) ?? null;
  }

  key(index) {
    return Array.from(this.store.keys())[index] ?? null;
  }

  setItem(key, value) {
    this.store.set(key, String(value));
  }

  removeItem(key) {
    this.store.delete(key);
  }

  clear() {
    this.store.clear();
  }
}

const AGENT = "a".repeat(64);
const REQUEST = {
  type: "agent_management_request",
  action: "create",
  requestId: "request-1",
  request: {
    channelId: "channel-1",
    displayName: "Draft Agent",
    systemPrompt: "You are concise.",
  },
};

function installStorage() {
  const storage = new MemoryStorage();
  globalThis.localStorage = storage;
  globalThis.window = { localStorage: storage };
  resetAgentManagementDraftStore();
  initAgentManagementDraftStore("owner-pubkey", "wss://relay.example.com");
}

test("enqueue dedupes by request id and notifies subscribers once", () => {
  installStorage();
  let notifications = 0;
  const unsubscribe = _testAgentManagementDraftStore.subscribeToStore(() => {
    notifications += 1;
  });

  assert.equal(
    enqueueAgentManagementDraft(
      AGENT,
      REQUEST,
      new Date("2026-07-23T00:00:00Z"),
    ),
    true,
  );
  assert.equal(
    enqueueAgentManagementDraft(
      AGENT,
      REQUEST,
      new Date("2026-07-23T00:00:01Z"),
    ),
    false,
  );

  assert.equal(getPendingAgentManagementDraftCount(), 1);
  assert.equal(notifications, 1);
  unsubscribe();
});

test("pending drafts are returned oldest first", () => {
  installStorage();
  const second = {
    ...REQUEST,
    requestId: "request-2",
    request: { ...REQUEST.request, displayName: "Second Agent" },
  };

  enqueueAgentManagementDraft(AGENT, second, new Date("2026-07-23T00:00:02Z"));
  enqueueAgentManagementDraft(AGENT, REQUEST, new Date("2026-07-23T00:00:01Z"));

  assert.equal(
    peekPendingAgentManagementDraft()?.request.requestId,
    "request-1",
  );
  assert.deepEqual(
    getPendingAgentManagementDrafts().map((draft) => draft.request.requestId),
    ["request-1", "request-2"],
  );
});

test("pending drafts persist per relay and owner scope", () => {
  installStorage();
  enqueueAgentManagementDraft(AGENT, REQUEST, new Date("2026-07-23T00:00:00Z"));

  resetAgentManagementDraftStore();
  initAgentManagementDraftStore("owner-pubkey", "wss://relay.example.com/");
  assert.equal(getPendingAgentManagementDraftCount(), 1);

  resetAgentManagementDraftStore();
  initAgentManagementDraftStore("owner-pubkey", "wss://other.example.com");
  assert.equal(getPendingAgentManagementDraftCount(), 0);
});

test("remove resolves one draft without clearing others", () => {
  installStorage();
  const second = {
    ...REQUEST,
    requestId: "request-2",
    request: { ...REQUEST.request, displayName: "Second Agent" },
  };
  enqueueAgentManagementDraft(AGENT, REQUEST, new Date("2026-07-23T00:00:00Z"));
  enqueueAgentManagementDraft(AGENT, second, new Date("2026-07-23T00:00:01Z"));

  assert.equal(removeAgentManagementDraft("request-1"), true);
  assert.equal(removeAgentManagementDraft("missing"), false);
  assert.deepEqual(
    getPendingAgentManagementDrafts().map((draft) => draft.request.requestId),
    ["request-2"],
  );
});
