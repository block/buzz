import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { formatWorkingTooltip } from "./SidebarSection.tsx";

function summary(agentNames, agentCount = agentNames.length) {
  return {
    channelId: "chan-1",
    anchorAt: 0,
    agentCount,
    agentPubkeys: agentNames.map((name) => `${name.toLowerCase()}-pubkey`),
    agentNames,
  };
}

describe("formatWorkingTooltip", () => {
  it("lists one agent name", () => {
    assert.equal(formatWorkingTooltip(summary(["Ned"])), "Ned working");
  });

  it("joins two agent names with an ampersand", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart"])),
      "Ned & Bart working",
    );
  });

  it("lists up to three agent names", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl"])),
      "Ned, Bart, & Carl working",
    );
  });

  it("uses the others count after three agent names", () => {
    assert.equal(
      formatWorkingTooltip(summary(["Ned", "Bart", "Carl", "Marge", "Lisa"])),
      "Ned, Bart, Carl, & 2 others working",
    );
  });
});
