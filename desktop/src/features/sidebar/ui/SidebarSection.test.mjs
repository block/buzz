import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { formatWorkingTooltip } from "./SidebarSection.tsx";

function summary(agentNames, agentCount = agentNames.length) {
  return {
    channelId: "chan-1",
    anchorAt: 0,
    agentCount,
    agentPubkeys: agentNames.map(
      (name, index) => `${name.toLowerCase()}-${index}-pubkey`,
    ),
    agentNames,
  };
}

describe("formatWorkingTooltip", () => {
  it("lists one agent name", () => {
    assert.equal(formatWorkingTooltip(summary(["Ned"])), "Ned working");
  });

  it("joins two agent names with and", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart"])),
      "Ned and Bart working",
    );
  });

  it("lists up to three agent names", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl"])),
      "Ned, Bart, and Carl working",
    );
  });

  it("uses the more count after three agent names", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl", "Marge", "Lisa"])),
      "Ned, Bart, Carl, and 2 more working",
    );
  });

  it("uses and 1 more for the four-agent boundary", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl", "Marge"])),
      "Ned, Bart, Carl, and 1 more working",
    );
  });

  it("uses a generic label for one unresolved agent", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "another agent"])),
      "Ned and another agent working",
    );
  });

  it("collapses multiple unresolved agents into the more count", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "another agent", "another agent"])),
      "Ned and 2 more working",
    );
  });
});
