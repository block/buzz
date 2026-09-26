import assert from "node:assert/strict";
import { describe, test } from "node:test";

import {
  buildSkillInsertion,
  extractAvailableAgentSkills,
} from "./composerAgentSkills.ts";

function event(seq, overrides = {}) {
  return {
    seq,
    timestamp: `2026-08-22T00:00:0${seq}.000Z`,
    kind: "acp_read",
    agentIndex: 0,
    channelId: "channel-a",
    sessionId: "session-a",
    turnId: "turn-a",
    payload: {},
    ...overrides,
  };
}

function commandsEvent(seq, commands, overrides = {}) {
  return event(seq, {
    payload: {
      method: "session/update",
      params: {
        update: {
          sessionUpdate: "available_commands_update",
          availableCommands: commands,
        },
      },
    },
    ...overrides,
  });
}

describe("extractAvailableAgentSkills", () => {
  test("returns the latest commands for the current channel and session", () => {
    const skills = extractAvailableAgentSkills(
      [
        commandsEvent(1, [{ name: "old", description: "Old command" }]),
        commandsEvent(
          2,
          [
            { name: "/review", description: "Review this change" },
            { name: "plan", description: "Create a plan" },
            { name: "plan", description: "Duplicate" },
          ],
          { sessionId: "session-b" },
        ),
      ],
      "channel-a",
    );

    assert.deepEqual(skills, [
      {
        name: "review",
        description: "Review this change",
        inputHint: "",
      },
      { name: "plan", description: "Create a plan", inputHint: "" },
    ]);
  });

  test("does not leak commands from a previous session", () => {
    const skills = extractAvailableAgentSkills(
      [
        commandsEvent(1, [{ name: "review" }]),
        event(2, { sessionId: "session-b", kind: "turn_start" }),
      ],
      "channel-a",
    );

    assert.deepEqual(skills, []);
  });

  test("keeps commands advertised while a new session id is still unknown", () => {
    const skills = extractAvailableAgentSkills(
      [
        commandsEvent(1, [{ name: "review" }], { sessionId: null }),
        event(2, { kind: "session_resolved" }),
      ],
      "channel-a",
    );

    assert.deepEqual(
      skills.map((skill) => skill.name),
      ["review"],
    );
  });

  test("ignores command updates from other channels", () => {
    const skills = extractAvailableAgentSkills(
      [
        commandsEvent(1, [{ name: "review" }]),
        commandsEvent(2, [{ name: "wrong" }], {
          channelId: "channel-b",
          sessionId: "session-b",
        }),
      ],
      "channel-a",
    );

    assert.deepEqual(
      skills.map((skill) => skill.name),
      ["review"],
    );
  });
});

describe("buildSkillInsertion", () => {
  test("inserts a slash command at an empty caret", () => {
    assert.deepEqual(buildSkillInsertion("", 0, "review"), {
      insertText: "/review ",
      replaceFromOffset: 0,
      replaceToOffset: 0,
    });
  });

  test("adds spacing when inserting between words", () => {
    assert.deepEqual(buildSkillInsertion("helloworld", 5, "/review"), {
      insertText: " /review ",
      replaceFromOffset: 5,
      replaceToOffset: 5,
    });
  });

  test("does not duplicate existing whitespace", () => {
    assert.deepEqual(buildSkillInsertion("hello world", 6, "review"), {
      insertText: "/review ",
      replaceFromOffset: 6,
      replaceToOffset: 6,
    });
  });

  test("rejects an invalid command name", () => {
    assert.equal(buildSkillInsertion("", 0, "two words"), null);
  });
});
