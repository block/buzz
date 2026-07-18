import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { applyAcpSessionScopeSetting } from "./acpSessionScopeSetting.ts";

const localRunning = {
  pubkey: "local",
  status: "running",
  backend: { type: "local" },
};
const remoteRunning = {
  pubkey: "remote",
  status: "running",
  backend: { type: "remote" },
};
const localStopped = {
  pubkey: "stopped",
  status: "stopped",
  backend: { type: "local" },
};

function harness(overrides = {}) {
  const calls = [];
  return {
    calls,
    deps: {
      setBackend: async (scope) => calls.push(["backend", scope]),
      listAgents: async () => [localRunning, remoteRunning, localStopped],
      stopAgent: async (pubkey) => calls.push(["stop", pubkey]),
      startAgent: async (pubkey) => calls.push(["start", pubkey]),
      setUi: (enabled) => calls.push(["ui", enabled]),
      ...overrides,
    },
  };
}

describe("ACP session scope setting", () => {
  it("commits UI only after applying backend state and restarting running local agents", async () => {
    const { calls, deps } = harness();
    await applyAcpSessionScopeSetting(false, true, deps);
    assert.deepEqual(calls, [
      ["backend", "thread"],
      ["stop", "local"],
      ["start", "local"],
      ["ui", true],
    ]);
  });

  it("rolls backend, processes, and UI back when restart fails", async () => {
    let starts = 0;
    const { calls, deps } = harness({
      startAgent: async (pubkey) => {
        calls.push(["start", pubkey]);
        starts += 1;
        if (starts === 1) throw new Error("restart failed");
      },
    });
    await assert.rejects(
      applyAcpSessionScopeSetting(false, true, deps),
      /restart failed/,
    );
    assert.deepEqual(calls, [
      ["backend", "thread"],
      ["stop", "local"],
      ["start", "local"],
      ["backend", "channel"],
      ["stop", "local"],
      ["start", "local"],
      ["ui", false],
    ]);
  });

  it("rolls UI back when persistence fails before any restart", async () => {
    const { calls, deps } = harness({
      setBackend: async (scope) => {
        calls.push(["backend", scope]);
        if (scope === "thread") throw new Error("persist failed");
      },
    });
    await assert.rejects(
      applyAcpSessionScopeSetting(false, true, deps),
      /persist failed/,
    );
    assert.equal(calls.at(-1)[0], "ui");
    assert.equal(calls.at(-1)[1], false);
  });

  it("attempts every process rollback even when one rollback restart fails", async () => {
    const first = {
      pubkey: "first",
      status: "running",
      backend: { type: "local" },
    };
    const second = {
      pubkey: "second",
      status: "running",
      backend: { type: "local" },
    };
    let firstStarts = 0;
    let secondStarts = 0;
    const { calls, deps } = harness({
      listAgents: async () => [first, second],
      startAgent: async (pubkey) => {
        calls.push(["start", pubkey]);
        if (pubkey === "first") {
          firstStarts += 1;
          if (firstStarts === 2) throw new Error("first rollback failed");
        } else {
          secondStarts += 1;
          if (secondStarts === 1) throw new Error("apply failed");
        }
      },
    });

    await assert.rejects(
      applyAcpSessionScopeSetting(false, true, deps),
      /apply failed/,
    );
    assert.deepEqual(calls, [
      ["backend", "thread"],
      ["stop", "first"],
      ["start", "first"],
      ["stop", "second"],
      ["start", "second"],
      ["backend", "channel"],
      ["stop", "first"],
      ["start", "first"],
      ["stop", "second"],
      ["start", "second"],
      ["ui", false],
    ]);
    assert.equal(secondStarts, 2);
  });
});
