import assert from "node:assert/strict";
import test from "node:test";

import { shouldProvisionLocalStarterTeam } from "./starterTeamPolicy.ts";

test("community creators receive the local Buzz starter team", () => {
  assert.equal(shouldProvisionLocalStarterTeam(undefined), true);
  assert.equal(shouldProvisionLocalStarterTeam(null), true);
  assert.equal(shouldProvisionLocalStarterTeam(""), true);
});

test("invited members inherit the community roster instead of local starter agents", () => {
  assert.equal(shouldProvisionLocalStarterTeam("invite-code"), false);
  assert.equal(shouldProvisionLocalStarterTeam("  invite-code  "), false);
});
