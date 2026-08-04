import assert from "node:assert/strict";
import test from "node:test";

import {
  runtimeCanReceiveMessages,
  startManagedAgentRuntimeAndWait,
} from "./managedAgentRuntimeReadiness.ts";

const pubkey = "a".repeat(64);
const relayUrl = "wss://relay.example.com";

function runtime(lifecycle, error = null) {
  return {
    pubkey,
    relayUrl,
    localSetup: true,
    lifecycle,
    pid: lifecycle === "stopped" ? null : 42,
    error,
    logPath: null,
  };
}

test("receive-ready lifecycle excludes a merely spawned process", () => {
  assert.equal(runtimeCanReceiveMessages("starting"), false);
  assert.equal(runtimeCanReceiveMessages("listening"), true);
  assert.equal(runtimeCanReceiveMessages("waking"), true);
  assert.equal(runtimeCanReceiveMessages("ready"), true);
});

test("message-triggered start waits until the runtime is listening", async () => {
  let now = 0;
  let listCalls = 0;
  const result = await startManagedAgentRuntimeAndWait({
    agentName: "Fizz",
    pubkey,
    relayUrl,
    pollIntervalMs: 10,
    timeoutMs: 100,
    dependencies: {
      startRuntime: async () => runtime("starting"),
      listRuntimes: async () => {
        listCalls += 1;
        return [runtime(listCalls === 1 ? "starting" : "listening")];
      },
      now: () => now,
      sleep: async (milliseconds) => {
        now += milliseconds;
      },
    },
  });

  assert.equal(result.lifecycle, "listening");
  assert.equal(listCalls, 2);
});

test("message-triggered start surfaces a runtime failure", async () => {
  await assert.rejects(
    startManagedAgentRuntimeAndWait({
      agentName: "Fizz",
      pubkey,
      relayUrl,
      dependencies: {
        startRuntime: async () => runtime("failed", "relay auth rejected"),
        listRuntimes: async () => [],
        now: () => 0,
        sleep: async () => {},
      },
    }),
    /Fizz could not start: relay auth rejected/,
  );
});

test("message-triggered start times out instead of sending silently", async () => {
  let now = 0;
  await assert.rejects(
    startManagedAgentRuntimeAndWait({
      agentName: "Fizz",
      pubkey,
      relayUrl,
      pollIntervalMs: 10,
      timeoutMs: 20,
      dependencies: {
        startRuntime: async () => runtime("starting"),
        listRuntimes: async () => [runtime("starting")],
        now: () => now,
        sleep: async (milliseconds) => {
          now += milliseconds;
        },
      },
    }),
    /Fizz did not become ready within 1 seconds/,
  );
});
