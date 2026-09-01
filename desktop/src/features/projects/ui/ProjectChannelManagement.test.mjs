import assert from "node:assert/strict";
import test from "node:test";

import { canManageProjectChannels } from "./ProjectChannelManagement.tsx";

const OWNER = "a".repeat(64);
const VIEWER = "b".repeat(64);

function project(overrides = {}) {
  return {
    legacy: false,
    owner: OWNER,
    ...overrides,
  };
}

test("Project owner and home-channel owner/admin can manage related channels", () => {
  assert.equal(canManageProjectChannels(project(), OWNER), true);
  assert.equal(canManageProjectChannels(project(), VIEWER, "owner"), true);
  assert.equal(canManageProjectChannels(project(), VIEWER, "admin"), true);
  assert.equal(
    canManageProjectChannels(project(), VIEWER, undefined, true),
    true,
  );
});

test("members, guests, bots, unrelated identities, and legacy Projects cannot manage channels", () => {
  for (const role of ["member", "guest", "bot", undefined]) {
    assert.equal(canManageProjectChannels(project(), VIEWER, role), false);
  }
  assert.equal(
    canManageProjectChannels(project({ legacy: true }), OWNER, "owner"),
    false,
  );
});
