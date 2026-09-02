import assert from "node:assert/strict";
import test from "node:test";

import { resolveProjectChannelManagement } from "./useCanManageProjectChannels.ts";

const base = {
  activeHomeChannel: true,
  homeCanManage: false,
  projectIsLegacy: false,
  projectOwner: "a".repeat(64),
  projectOwnerIsManaged: false,
  viewerIsProjectOwner: false,
  viewerOwnsProjectAgent: false,
};

test("an active home-channel administrator can manage related channels", () => {
  const capabilities = resolveProjectChannelManagement({
    ...base,
    homeCanManage: true,
  });
  assert.equal(capabilities.canManage, true);
  assert.equal(capabilities.canCreate, false);
});

test("an ordinary home-channel member cannot manage related channels", () => {
  const capabilities = resolveProjectChannelManagement(base);
  assert.equal(capabilities.canManage, false);
});

test("channel authority is unavailable without an active Project home", () => {
  const capabilities = resolveProjectChannelManagement({
    ...base,
    activeHomeChannel: false,
    homeCanManage: true,
  });
  assert.equal(capabilities.canManage, false);
});

test("the Project owner can manage without a home channel", () => {
  const capabilities = resolveProjectChannelManagement({
    ...base,
    activeHomeChannel: false,
    viewerIsProjectOwner: true,
  });
  assert.equal(capabilities.canManage, true);
  assert.equal(capabilities.canCreate, true);
});
