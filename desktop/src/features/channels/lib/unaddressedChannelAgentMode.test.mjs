import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_UNADDRESSED_CHANNEL_AGENT_MODE,
  UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY,
  parseUnaddressedChannelAgentMode,
  readUnaddressedChannelAgentMode,
  writeUnaddressedChannelAgentMode,
} from "./unaddressedChannelAgentMode.ts";

test("storage key matches fixture contract", () => {
  assert.equal(
    UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY,
    "buzz:unaddressed-channel-agent-mode:v1",
  );
});

test("default mode is all-channel-agents", () => {
  assert.equal(DEFAULT_UNADDRESSED_CHANNEL_AGENT_MODE, "all-channel-agents");
  assert.equal(parseUnaddressedChannelAgentMode(null), "all-channel-agents");
  assert.equal(
    parseUnaddressedChannelAgentMode("garbage"),
    "all-channel-agents",
  );
});

test("parse accepts both modes", () => {
  assert.equal(
    parseUnaddressedChannelAgentMode("all-channel-agents"),
    "all-channel-agents",
  );
  assert.equal(
    parseUnaddressedChannelAgentMode("mentions-only"),
    "mentions-only",
  );
});

test("read/write round-trip via mock storage", () => {
  const map = new Map();
  const storage = {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => {
      map.set(k, v);
    },
  };
  assert.equal(readUnaddressedChannelAgentMode(storage), "all-channel-agents");
  writeUnaddressedChannelAgentMode("mentions-only", storage);
  assert.equal(readUnaddressedChannelAgentMode(storage), "mentions-only");
  assert.equal(
    map.get(UNADDRESSED_CHANNEL_AGENT_MODE_STORAGE_KEY),
    "mentions-only",
  );
});
