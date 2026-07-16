import assert from "node:assert/strict";
import test from "node:test";

import {
  clearCommunityOnboardingTransaction,
  loadCommunityOnboardingTransaction,
  markCommunityOnboardingComplete,
  startCommunityOnboarding,
  updateCommunityOnboardingTransaction,
} from "./communityOnboarding.tsx";

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  };
}

test("invite onboarding starts at claim and normalizes its relay", () => {
  const storage = createMemoryStorage();
  const transaction = startCommunityOnboarding(
    {
      source: "deep-link-join",
      relayUrl: "WSS://Relay.Example/path/",
      inviteCode: "  invite-code  ",
    },
    storage,
    new Date("2026-07-16T00:00:00Z"),
  );
  assert.equal(transaction.stage, "claiming");
  assert.equal(transaction.relayUrl, "wss://relay.example/path");
  assert.equal(transaction.inviteCode, "invite-code");
  const persisted = loadCommunityOnboardingTransaction(storage);
  assert.equal(persisted?.id, transaction.id);
  assert.equal(persisted?.stage, transaction.stage);
  assert.equal(persisted?.relayUrl, transaction.relayUrl);
});

test("non-invite onboarding starts at connection", () => {
  const transaction = startCommunityOnboarding(
    { source: "add-community", relayUrl: "wss://relay.example" },
    createMemoryStorage(),
  );
  assert.equal(transaction.stage, "connecting");
});

test("same-relay ingress resumes rather than replacing progress", () => {
  const storage = createMemoryStorage();
  const first = startCommunityOnboarding(
    { source: "add-community", relayUrl: "wss://relay.example" },
    storage,
    new Date("2026-07-16T00:00:00Z"),
  );
  const progressed = updateCommunityOnboardingTransaction(
    first,
    { stage: "profile", communityId: "community-id" },
    storage,
    new Date("2026-07-16T00:01:00Z"),
  );
  const resumed = startCommunityOnboarding(
    {
      source: "deep-link-join",
      relayUrl: "wss://relay.example/",
      inviteCode: "new-code",
    },
    storage,
    new Date("2026-07-16T00:02:00Z"),
  );
  assert.equal(resumed.id, progressed.id);
  assert.equal(resumed.stage, "profile");
  assert.equal(resumed.communityId, "community-id");
  assert.equal(resumed.inviteCode, "new-code");
});

test("malformed persisted state is ignored and can be cleared", () => {
  const storage = createMemoryStorage({
    "buzz-community-onboarding-transaction.v1": '{"stage":"profile"}',
  });
  assert.equal(loadCommunityOnboardingTransaction(storage), null);
  clearCommunityOnboardingTransaction(storage);
  assert.equal(storage.length, 0);
});

test("completion is scoped by relay and pubkey and preserves legacy gate", () => {
  const storage = createMemoryStorage();
  markCommunityOnboardingComplete("pubkey", "wss://relay.example", storage);
  assert.equal(
    storage.getItem(
      "buzz-community-onboarding-complete.v1:wss%3A%2F%2Frelay.example:pubkey",
    ),
    "true",
  );
  assert.equal(storage.getItem("buzz-onboarding-complete.v1:pubkey"), "true");
});
