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
      getBackend: async () => {
        calls.push(["read-backend"]);
        return "thread";
      },
      listAgents: async () => [localRunning, remoteRunning, localStopped],
      stopAgent: async (pubkey) => calls.push(["stop", pubkey]),
      startAgent: async (pubkey) => calls.push(["start", pubkey]),
      setUi: (enabled) => calls.push(["ui", enabled]),
      onUnrecoverable: () => calls.push(["unrecoverable"]),
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

  it("attempts every process rollback and surfaces hard recovery when one rollback restart fails", async () => {
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
    // Every process rollback is still attempted, but a failed reconciliation
    // must surface hard recovery instead of claiming a normal scope.
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
      ["unrecoverable"],
    ]);
    assert.equal(secondStarts, 2);
    assert.ok(!calls.some((c) => c[0] === "ui"));
  });

  it("surfaces hard recovery when a rollback stop fails", async () => {
    let stops = 0;
    const { calls, deps } = harness({
      stopAgent: async (pubkey) => {
        calls.push(["stop", pubkey]);
        stops += 1;
        if (stops === 2) throw new Error("rollback stop failed");
      },
      startAgent: async (pubkey) => {
        calls.push(["start", pubkey]);
        if (calls.filter((c) => c[0] === "start").length === 1)
          throw new Error("apply failed");
      },
    });
    await assert.rejects(
      applyAcpSessionScopeSetting(false, true, deps),
      /apply failed/,
    );
    // The process may still be running under the wrong scope: no UI claim.
    assert.ok(calls.some((c) => c[0] === "unrecoverable"));
    assert.ok(!calls.some((c) => c[0] === "ui"));
  });

  it("reconciles UI and processes to the re-read authoritative scope when rollback persistence fails", async () => {
    let backendCalls = 0;
    const { calls, deps } = harness({
      setBackend: async (scope) => {
        calls.push(["backend", scope]);
        backendCalls += 1;
        if (backendCalls === 1) return; // apply write succeeds (thread persisted)
        throw new Error("rollback persist failed");
      },
      startAgent: async (pubkey) => {
        calls.push(["start", pubkey]);
        if (calls.filter((c) => c[0] === "start").length === 1)
          throw new Error("restart failed");
      },
      // Authoritative backend remains the NEW value (thread): the apply
      // write landed and the rollback write failed.
      getBackend: async () => {
        calls.push(["read-backend"]);
        return "thread";
      },
    });
    await assert.rejects(
      applyAcpSessionScopeSetting(false, true, deps),
      /restart failed/,
    );
    // UI must land on the actual persisted scope (thread), never the
    // assumed previous (channel), and processes reconcile after the read.
    const readIndex = calls.findIndex((c) => c[0] === "read-backend");
    const uiIndex = calls.findIndex((c) => c[0] === "ui");
    assert.notEqual(readIndex, -1);
    assert.deepEqual(calls[uiIndex], ["ui", true]);
    assert.ok(readIndex < uiIndex, "authority read must precede UI commit");
    const restartsAfterRead = calls
      .slice(readIndex)
      .filter((c) => c[0] === "stop" || c[0] === "start");
    assert.deepEqual(restartsAfterRead, [
      ["stop", "local"],
      ["start", "local"],
    ]);
    assert.ok(!calls.some((c) => c[0] === "unrecoverable"));
  });

  it("surfaces hard recovery and touches nothing when rollback and authority read both fail", async () => {
    let backendCalls = 0;
    const { calls, deps } = harness({
      setBackend: async (scope) => {
        calls.push(["backend", scope]);
        backendCalls += 1;
        if (backendCalls === 1) return;
        throw new Error("rollback persist failed");
      },
      startAgent: async (pubkey) => {
        calls.push(["start", pubkey]);
        if (calls.filter((c) => c[0] === "start").length === 1)
          throw new Error("restart failed");
      },
      getBackend: async () => {
        calls.push(["read-backend"]);
        throw new Error("authority unreadable");
      },
    });
    await assert.rejects(
      applyAcpSessionScopeSetting(false, true, deps),
      /restart failed/,
    );
    assert.ok(calls.some((c) => c[0] === "unrecoverable"));
    // No UI claim and no process restarts under an unknown scope.
    const readIndex = calls.findIndex((c) => c[0] === "read-backend");
    assert.ok(!calls.some((c) => c[0] === "ui"));
    assert.deepEqual(
      calls
        .slice(readIndex + 1)
        .filter((c) => c[0] === "stop" || c[0] === "start"),
      [],
    );
  });
});
