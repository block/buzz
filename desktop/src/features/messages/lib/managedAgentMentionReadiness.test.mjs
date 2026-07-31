import assert from "node:assert/strict";
import test from "node:test";

import {
  isLiveAgentPresence,
  shouldStartManagedAgentForMention,
} from "./managedAgentMentionReadiness.ts";

const localAgent = (status) => ({
  backend: { type: "local" },
  status,
});

const providerAgent = (status) => ({
  backend: { type: "provider", id: "remote", config: {} },
  status,
});

test("online or away presence is a live external-writer signal", () => {
  assert.equal(isLiveAgentPresence("online"), true);
  assert.equal(isLiveAgentPresence("away"), true);
  assert.equal(isLiveAgentPresence("offline"), false);
  assert.equal(isLiveAgentPresence(undefined), false);
});

test("does not start a stopped local record while the identity is live remotely", () => {
  assert.equal(
    shouldStartManagedAgentForMention(localAgent("stopped"), "online"),
    false,
  );
  assert.equal(
    shouldStartManagedAgentForMention(localAgent("stopped"), "away"),
    false,
  );
});

test("starts an inactive local record when no live writer is present", () => {
  assert.equal(
    shouldStartManagedAgentForMention(localAgent("stopped"), "offline"),
    true,
  );
  assert.equal(
    shouldStartManagedAgentForMention(localAgent("stopped"), undefined),
    true,
  );
});

test("keeps already active local and provider records idempotent", () => {
  assert.equal(
    shouldStartManagedAgentForMention(localAgent("running"), "offline"),
    false,
  );
  assert.equal(
    shouldStartManagedAgentForMention(providerAgent("deployed"), "offline"),
    false,
  );
  assert.equal(
    shouldStartManagedAgentForMention(providerAgent("not_deployed"), "offline"),
    true,
  );
});
