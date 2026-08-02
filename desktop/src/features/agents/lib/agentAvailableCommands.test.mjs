import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  normalizeAvailableCommands,
  parseAvailableCommandsEvent,
} from "./agentAvailableCommands.ts";

function observerEvent(payload, overrides = {}) {
  return {
    seq: 1,
    timestamp: "2026-08-02T00:00:00Z",
    kind: "acp_read",
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: "turn-1",
    payload,
    ...overrides,
  };
}

describe("available agent commands", () => {
  it("normalizes object and string commands and removes duplicates", () => {
    assert.deepEqual(
      normalizeAvailableCommands([
        {
          name: "/ad-monitor",
          description: " Review the latest ads ",
          inputHint: "[period]",
        },
        "creative-run",
        { name: "AD-MONITOR", description: "duplicate" },
        { name: "not a command" },
        null,
      ]),
      [
        {
          name: "ad-monitor",
          description: "Review the latest ads",
          inputHint: "[period]",
        },
        {
          name: "creative-run",
          description: null,
          inputHint: null,
        },
      ],
    );
  });

  it("parses ACP available_commands_update events", () => {
    const parsed = parseAvailableCommandsEvent(
      observerEvent({
        method: "session/update",
        params: {
          update: {
            sessionUpdate: "available_commands_update",
            availableCommands: [
              { name: "ad-monitor", description: "Check ads" },
            ],
          },
        },
      }),
    );

    assert.deepEqual(parsed, [
      {
        name: "ad-monitor",
        description: "Check ads",
        inputHint: null,
      },
    ]);
  });

  it("distinguishes an empty catalog update from unrelated events", () => {
    assert.deepEqual(
      parseAvailableCommandsEvent(
        observerEvent({
          method: "session/update",
          params: {
            update: {
              sessionUpdate: "available_commands_update",
              availableCommands: [],
            },
          },
        }),
      ),
      [],
    );
    assert.equal(
      parseAvailableCommandsEvent(observerEvent({ method: "session/prompt" })),
      null,
    );
  });
});
