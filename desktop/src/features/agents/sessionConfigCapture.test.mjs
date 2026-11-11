/**
 * A captured session config is stored only from live observer events, and the
 * config-surface refresh waits for that store so readers see the new options.
 */
import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  _testProcessLiveObserverEvents,
  _testRegisterKnownAgents,
  ingestArchivedObserverEvents,
  resetAgentObserverStore,
  setSessionConfigCapturedCallback,
} from "@/features/agents/observerRelayStore.ts";

const AGENT = "a".repeat(64);
const log = [];
let settleStore = () => {};

globalThis.window ??= globalThis;
globalThis.window.__TAURI_INTERNALS__ = {
  invoke: (cmd) => {
    log.push(cmd);
    if (cmd !== "put_agent_session_config") return Promise.resolve(null);
    return new Promise((resolve) => {
      settleStore = () => {
        log.push("stored");
        resolve(null);
      };
    });
  },
  transformCallback: () => 0,
};

const captured = {
  seq: 1,
  timestamp: "2026-01-01T00:00:01.000Z",
  kind: "session_config_captured",
  agentIndex: 0,
  channelId: "chan-1",
  sessionId: "sess-1",
  turnId: null,
  payload: { configOptions: [] },
};

beforeEach(() => {
  resetAgentObserverStore();
  log.length = 0;
  setSessionConfigCapturedCallback(() => log.push("refresh"));
});

test("live capture refreshes only after the store write settles", async () => {
  _testProcessLiveObserverEvents(AGENT, [captured]);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(log, ["put_agent_session_config"]);

  settleStore();
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(log, ["put_agent_session_config", "stored", "refresh"]);
});

test("archived capture is read-only: no store write and no refresh", async () => {
  _testRegisterKnownAgents("sub", [AGENT]);
  const raw = {
    id: "e".repeat(64),
    pubkey: AGENT,
    created_at: 1000,
    kind: 24200,
    tags: [
      ["p", "b".repeat(64)],
      ["agent", AGENT],
      ["frame", "telemetry"],
    ],
    content: "encrypted",
    sig: "s".repeat(128),
  };
  for (const event of [captured, { ...captured, channelId: null }]) {
    await ingestArchivedObserverEvents([raw], () => Promise.resolve(event));
  }
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(log, []);
});
