import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAgentCommandCatalog,
  parseAvailableCommandsPayload,
  recordAvailableCommandsUpdate,
  resetAgentCommandCatalogForTests,
} from "./agentCommandCatalog.ts";

const OWNER = "aa".repeat(32);
const OTHER_OWNER = "bb".repeat(32);
const AGENT = "cc".repeat(32);

function installLocalStorage() {
  const values = new Map();
  globalThis.window = {
    localStorage: {
      get length() {
        return values.size;
      },
      getItem: (key) => values.get(key) ?? null,
      key: (index) => [...values.keys()][index] ?? null,
      removeItem: (key) => values.delete(key),
      setItem: (key, value) => values.set(key, String(value)),
    },
  };
}

describe("agent command catalog", () => {
  beforeEach(() => {
    installLocalStorage();
    resetAgentCommandCatalogForTests();
  });

  it("sanitizes, bounds, and deduplicates advertised commands", () => {
    const commands = parseAvailableCommandsPayload({
      commands: [
        { name: "/review", description: " Review changes " },
        { name: "REVIEW", description: "duplicate" },
        { name: "bad name" },
        { name: "deploy", description: 42 },
      ],
    });

    assert.deepEqual(commands, [
      { name: "review", description: "Review changes" },
      { name: "deploy", description: null },
    ]);
  });

  it("keeps the latest complete command list per owner and agent", () => {
    assert.equal(
      recordAvailableCommandsUpdate(OWNER, AGENT, {
        seq: 8,
        timestamp: "2026-07-23T08:00:00Z",
        payload: { commands: [{ name: "review", description: "Review" }] },
      }),
      true,
    );
    assert.equal(
      recordAvailableCommandsUpdate(OWNER, AGENT, {
        seq: 7,
        timestamp: "2026-07-23T07:00:00Z",
        payload: { commands: [{ name: "stale" }] },
      }),
      false,
    );

    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT)?.commands, [
      { name: "review", description: "Review" },
    ]);
    assert.equal(getAgentCommandCatalog(OTHER_OWNER).has(AGENT), false);
  });

  it("treats an empty update as authoritative removal of prior commands", () => {
    recordAvailableCommandsUpdate(OWNER, AGENT, {
      seq: 1,
      timestamp: "2026-07-23T08:00:00Z",
      payload: { commands: [{ name: "review" }] },
    });
    recordAvailableCommandsUpdate(OWNER, AGENT, {
      seq: 2,
      timestamp: "2026-07-23T08:01:00Z",
      payload: { commands: [] },
    });

    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT)?.commands, []);
  });

  it("hydrates a persisted owner-scoped catalog after restart", () => {
    recordAvailableCommandsUpdate(OWNER, AGENT, {
      seq: 3,
      timestamp: "2026-07-23T08:00:00Z",
      payload: { commands: [{ name: "review" }] },
    });
    resetAgentCommandCatalogForTests();

    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT)?.commands, [
      { name: "review", description: null },
    ]);
  });
});
