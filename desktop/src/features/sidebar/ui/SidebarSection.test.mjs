import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { formatWorkingTooltip } from "./SidebarSection.tsx";

function summary(agentNames, agentCount = agentNames.length) {
  return {
    channelId: "chan-1",
    anchorAt: 0,
    agentCount,
    agentPubkeys: Array.from(
      { length: agentCount },
      (_, index) => `agent-${index}-pubkey`,
    ),
    agentNames,
  };
}

describe("formatWorkingTooltip", () => {
  it("names one known agent", () => {
    assert.equal(formatWorkingTooltip(summary(["Ned"])), "Ned working");
  });

  it("names one known agent and counts one additional agent", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart"])),
      "Ned and 1 agent working",
    );
  });

  it("names one known agent and counts multiple additional agents", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl"])),
      "Ned and 2 agents working",
    );
  });

  it("uses a singular count when all agents are unknown", () => {
    assert.equal(formatWorkingTooltip(summary([], 1)), "1 agent working");
  });

  it("uses a plural count when all agents are unknown", () => {
    assert.equal(formatWorkingTooltip(summary([], 3)), "3 agents working");
  });

  it("counts unknown agents with the named lead", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned"], 3)),
      "Ned and 2 agents working",
    );
  });
});
