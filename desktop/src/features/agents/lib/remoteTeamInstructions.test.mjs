import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  losesTeamInstructionsRemotely,
  REMOTE_TEAM_INSTRUCTIONS_ACTIVE_NOTICE,
  REMOTE_TEAM_INSTRUCTIONS_NOTICE,
} from "./remoteTeamInstructions.ts";

const provider = { type: "provider", id: "ssh", config: {} };
const local = { type: "local" };

describe("losesTeamInstructionsRemotely", () => {
  it("is true for a team-linked record that runs through a provider", () => {
    assert.equal(
      losesTeamInstructionsRemotely({ backend: provider, teamId: "team-1" }),
      true,
    );
  });

  it("is false for a team-linked record on this computer", () => {
    // Local spawn resolves the team's instructions and passes them to the
    // harness, so there is nothing to disclose.
    assert.equal(
      losesTeamInstructionsRemotely({ backend: local, teamId: "team-1" }),
      false,
    );
  });

  it("is false for a remote record with no team", () => {
    for (const teamId of [null, undefined, "", "   "]) {
      assert.equal(
        losesTeamInstructionsRemotely({ backend: provider, teamId }),
        false,
        `teamId ${JSON.stringify(teamId)}`,
      );
    }
  });

  it("is false when the record carries no backend yet", () => {
    assert.equal(
      losesTeamInstructionsRemotely({ backend: null, teamId: "team-1" }),
      false,
    );
    assert.equal(losesTeamInstructionsRemotely({ teamId: "team-1" }), false);
  });
});

describe("the disclosure copy", () => {
  it("names team instructions in both surfaces, so neither reads as generic", () => {
    for (const copy of [
      REMOTE_TEAM_INSTRUCTIONS_NOTICE,
      REMOTE_TEAM_INSTRUCTIONS_ACTIVE_NOTICE,
    ]) {
      assert.ok(/team/i.test(copy), copy);
      assert.ok(copy.trim().length > 0);
    }
  });
});
