import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import {
  _testProcessLiveObserverEvents,
  getAgentObserverSnapshot,
  resetAgentObserverStore,
} from "./observerRelayStore.ts";

const AGENT = "a".repeat(64);

function lifecycleEvent(seq, lifecycle, startNonce = "gen-1") {
  return {
    seq,
    timestamp: `2026-08-15T00:00:0${seq}Z`,
    kind: "managed_agent_runtime_lifecycle",
    agentIndex: 0,
    channelId: null,
    sessionId: null,
    turnId: null,
    payload: {
      relayUrl: "ws://localhost:3000",
      startNonce,
      lifecycle,
      pid: 123,
    },
  };
}

beforeEach(() => {
  resetAgentObserverStore();
});

afterEach(() => {
  delete globalThis.__TAURI_INTERNALS__;
  delete globalThis.window;
});

test("applies lifecycle frames in observer order", async () => {
  const started = [];
  const completed = [];
  let releaseWaking;

  const tauriInternals = {
    invoke: (command, args) => {
      assert.equal(command, "put_managed_agent_runtime_lifecycle");
      const lifecycle = args.payload.lifecycle;
      started.push(lifecycle);
      if (lifecycle === "waking") {
        return new Promise((resolve) => {
          releaseWaking = () => {
            completed.push(lifecycle);
            resolve({});
          };
        });
      }
      completed.push(lifecycle);
      return Promise.resolve({});
    },
  };
  globalThis.__TAURI_INTERNALS__ = tauriInternals;
  globalThis.window = { __TAURI_INTERNALS__: tauriInternals };

  const processing = _testProcessLiveObserverEvents(AGENT, [
    lifecycleEvent(1, "waking"),
    lifecycleEvent(2, "ready"),
  ]);
  await Promise.resolve();

  assert.deepEqual(
    started,
    ["waking"],
    "ready must wait for the preceding waking write",
  );
  releaseWaking();
  await processing;

  assert.deepEqual(started, ["waking", "ready"]);
  assert.deepEqual(completed, ["waking", "ready"]);
});

test("drops the remainder of a lifecycle batch after a store reset", async () => {
  const started = [];
  let releaseWaking;

  const tauriInternals = {
    invoke: (_command, args) => {
      const lifecycle = args.payload.lifecycle;
      started.push(lifecycle);
      if (lifecycle === "waking") {
        return new Promise((resolve) => {
          releaseWaking = resolve;
        });
      }
      return Promise.resolve({});
    },
  };
  globalThis.__TAURI_INTERNALS__ = tauriInternals;
  globalThis.window = { __TAURI_INTERNALS__: tauriInternals };

  const processing = _testProcessLiveObserverEvents(AGENT, [
    lifecycleEvent(1, "waking"),
    lifecycleEvent(2, "ready"),
  ]);
  await Promise.resolve();

  resetAgentObserverStore();
  releaseWaking();
  await processing;

  assert.deepEqual(started, ["waking"]);
  assert.deepEqual(getAgentObserverSnapshot(AGENT).events, []);
});

test("newest-first replay does not regress ready to stale waking", async () => {
  const started = [];
  const tauriInternals = {
    invoke: (command, args) => {
      assert.equal(command, "put_managed_agent_runtime_lifecycle");
      started.push(args.payload.lifecycle);
      return Promise.resolve({});
    },
  };
  globalThis.__TAURI_INTERNALS__ = tauriInternals;
  globalThis.window = { __TAURI_INTERNALS__: tauriInternals };

  await _testProcessLiveObserverEvents(AGENT, [lifecycleEvent(2, "ready")]);
  await _testProcessLiveObserverEvents(AGENT, [lifecycleEvent(1, "waking")]);

  assert.deepEqual(started, ["ready"]);
});

test("a new startNonce restarts the lifecycle sequence domain", async () => {
  const started = [];
  const tauriInternals = {
    invoke: (_command, args) => {
      started.push(`${args.payload.startNonce}:${args.payload.lifecycle}`);
      return Promise.resolve({});
    },
  };
  globalThis.__TAURI_INTERNALS__ = tauriInternals;
  globalThis.window = { __TAURI_INTERNALS__: tauriInternals };

  await _testProcessLiveObserverEvents(AGENT, [
    lifecycleEvent(2, "ready", "gen-1"),
  ]);
  await _testProcessLiveObserverEvents(AGENT, [
    lifecycleEvent(1, "waking", "gen-2"),
  ]);

  assert.deepEqual(started, ["gen-1:ready", "gen-2:waking"]);
});
