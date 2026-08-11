import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildAgentWorkingStatuses,
  formatAgentWorkingStatusLabel,
} from "./ThreadAgentWorkingIndicator.tsx";

const MARGARET = {
  pubkey: "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111",
  name: "Margaret",
};
const FIZZ = {
  pubkey: "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222",
  name: "Fizz",
};

describe("formatAgentWorkingStatusLabel", () => {
  it("uses thinking phrasing with no live headline", () => {
    assert.equal(
      formatAgentWorkingStatusLabel([
        { agent: MARGARET, status: "Thinking", headlines: [] },
      ]),
      "Margaret is thinking…",
    );
  });

  it("shows agent name and activity headline", () => {
    assert.equal(
      formatAgentWorkingStatusLabel([
        {
          agent: MARGARET,
          status: "Searching files",
          headlines: ["Searching files"],
        },
      ]),
      "Margaret: Searching files",
    );
  });

  it("rotates multi-agent focus with a +N suffix", () => {
    const statuses = [
      {
        agent: MARGARET,
        status: "Searching files",
        headlines: ["Searching files"],
      },
      { agent: FIZZ, status: "Thinking", headlines: [] },
    ];
    assert.equal(
      formatAgentWorkingStatusLabel(statuses, 0),
      "Margaret: Searching files · +1 agent",
    );
    assert.equal(
      formatAgentWorkingStatusLabel(statuses, 1),
      "Fizz is thinking… · +1 agent",
    );
  });
});

describe("buildAgentWorkingStatuses", () => {
  it("falls back to Thinking when transcript is empty", () => {
    const statuses = buildAgentWorkingStatuses([MARGARET], "chan-1", () => []);
    assert.deepEqual(statuses, [
      { agent: MARGARET, headlines: [], status: "Thinking" },
    ]);
  });
});
