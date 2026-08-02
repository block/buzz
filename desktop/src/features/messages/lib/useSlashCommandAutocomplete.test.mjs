import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  detectSlashCommandQuery,
  rankSlashCommands,
} from "./useSlashCommandAutocomplete.ts";

const commands = [
  {
    name: "creative-run",
    description: "Plan the next creative batch",
    inputHint: null,
  },
  {
    name: "ad-monitor",
    description: "Review live advertising performance",
    inputHint: null,
  },
  {
    name: "creative-learnings",
    description: "Extract durable ad patterns",
    inputHint: null,
  },
];

describe("slash command autocomplete", () => {
  it("detects an empty or partial slash token at the cursor", () => {
    assert.deepEqual(detectSlashCommandQuery("@Fizz /", 7), {
      query: "",
      startIndex: 6,
    });
    assert.deepEqual(detectSlashCommandQuery("@Fizz /ad", 9), {
      query: "ad",
      startIndex: 6,
    });
    assert.equal(detectSlashCommandQuery("look/inside", 11), null);
    assert.equal(detectSlashCommandQuery("/ad now", 7), null);
  });

  it("ranks names before descriptions and preserves source order", () => {
    assert.deepEqual(
      rankSlashCommands(commands, "ad").map((command) => command.name),
      ["ad-monitor", "creative-learnings"],
    );
    assert.deepEqual(
      rankSlashCommands(commands, "creative").map((command) => command.name),
      ["creative-run", "creative-learnings"],
    );
  });
});
