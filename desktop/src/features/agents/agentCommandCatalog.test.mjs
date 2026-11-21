import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAgentCommandCatalog,
  initAgentCommandCatalog,
  resetAgentCommandCatalog,
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
    initAgentCommandCatalog("test-community");
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

  it("rejects hidden controls in command names and removes them from descriptions", () => {
    assert.deepEqual(
      parseAvailableCommandsPayload({
        commands: [
          { name: "rev\u0000iew" },
          { name: "rev\u202eiew" },
          { name: "rev\u200biew" },
          { name: "review", description: "a\u001bb\u202ec" },
        ],
      }),
      [{ name: "review", description: "a b c" }],
    );
  });

  it("enforces count, name, and description bounds", () => {
    assert.equal(
      parseAvailableCommandsPayload({
        commands: Array.from({ length: 300 }, (_, i) => ({ name: `cmd-${i}` })),
      }).length,
      256,
    );
    assert.deepEqual(
      parseAvailableCommandsPayload({ commands: [{ name: "a".repeat(129) }] }),
      [],
    );
    assert.equal(
      parseAvailableCommandsPayload({
        commands: [{ name: "review", description: "a".repeat(600) }],
      })[0].description.length,
      512,
    );
  });

  it("ignores malformed snapshots and uses sequence to break timestamp ties", () => {
    const timestamp = "2026-07-23T08:00:00Z";
    recordAvailableCommandsUpdate(OWNER, AGENT, {
      seq: 2,
      timestamp,
      payload: { commands: [{ name: "review" }] },
    });
    for (const event of [
      { seq: 3, timestamp, payload: {} },
      { seq: 3, timestamp: "invalid", payload: { commands: [] } },
      { seq: 1, timestamp, payload: { commands: [] } },
    ])
      assert.equal(recordAvailableCommandsUpdate(OWNER, AGENT, event), false);
    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT).commands, [
      { name: "review", description: null },
    ]);
  });

  it("re-sanitizes persisted commands and tolerates unavailable storage", () => {
    window.localStorage.setItem(
      `buzz-agent-command-catalog.v1:test-community:${OWNER}`,
      JSON.stringify({
        version: 1,
        agents: {
          [AGENT]: {
            commands: [{ name: "bad\u202e" }, { name: "review" }],
            seq: 1,
            timestamp: "2026-07-23T08:00:00Z",
          },
        },
      }),
    );
    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT).commands, [
      { name: "review", description: null },
    ]);
    resetAgentCommandCatalogForTests();
    initAgentCommandCatalog("test-community");
    window.localStorage.getItem = () => {
      throw new Error("storage disabled");
    };
    assert.equal(getAgentCommandCatalog(OWNER).size, 0);
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
    initAgentCommandCatalog("test-community");

    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT)?.commands, [
      { name: "review", description: null },
    ]);
  });

  it("isolates the same owner and agent across community switches and restores on return", () => {
    const event = {
      seq: 1,
      timestamp: "2026-07-23T08:00:00Z",
      payload: { commands: [{ name: "review" }] },
    };
    recordAvailableCommandsUpdate(OWNER, AGENT, event);
    resetAgentCommandCatalog();
    assert.equal(getAgentCommandCatalog(OWNER).size, 0);
    assert.equal(recordAvailableCommandsUpdate(OWNER, AGENT, event), false);
    initAgentCommandCatalog("other-community");
    assert.equal(getAgentCommandCatalog(OWNER).size, 0);
    recordAvailableCommandsUpdate(OWNER, AGENT, {
      ...event,
      payload: { commands: [{ name: "deploy" }] },
    });
    initAgentCommandCatalog("test-community");
    assert.deepEqual(getAgentCommandCatalog(OWNER).get(AGENT).commands, [
      { name: "review", description: null },
    ]);
  });
});
