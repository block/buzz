import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";

import {
  getAvailableAgentCommands,
  injectObserverEventsForE2E,
  resetAgentObserverStore,
} from "./observerRelayStore.ts";

const AGENT = "a".repeat(64);

function commandEvent(seq, timestamp, names) {
  return {
    seq,
    timestamp,
    kind: "acp_read",
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: "turn-1",
    payload: {
      method: "session/update",
      params: {
        update: {
          sessionUpdate: "available_commands_update",
          availableCommands: names.map((name) => ({ name })),
        },
      },
    },
  };
}

describe("available commands observer store", () => {
  beforeEach(() => resetAgentObserverStore());

  it("keeps the latest runtime catalog for each agent", () => {
    injectObserverEventsForE2E(AGENT, [
      commandEvent(2, "2026-08-02T00:00:02Z", ["new-command"]),
      commandEvent(1, "2026-08-02T00:00:01Z", ["stale-command"]),
    ]);

    assert.deepEqual(
      getAvailableAgentCommands(AGENT).map((command) => command.name),
      ["new-command"],
    );
  });

  it("accepts a newer empty catalog and clears it on reset", () => {
    injectObserverEventsForE2E(AGENT, [
      commandEvent(1, "2026-08-02T00:00:01Z", ["ad-monitor"]),
      commandEvent(2, "2026-08-02T00:00:02Z", []),
    ]);
    assert.deepEqual(getAvailableAgentCommands(AGENT), []);

    resetAgentObserverStore();
    assert.deepEqual(getAvailableAgentCommands(AGENT), []);
  });
});
