import assert from "node:assert/strict";
import { test } from "node:test";

import {
  canManageProjectRelatedChannels,
  listLinkableProjectChannels,
} from "./projectRelatedChannelAccess.ts";

const OWNER = "a".repeat(64);
const ADMIN = "b".repeat(64);
const HOME = "11111111-1111-4111-8111-111111111111";
const RELATED = "22222222-2222-4222-8222-222222222222";
const REPOSITORY = "33333333-3333-4333-8333-333333333333";
const AVAILABLE = "44444444-4444-4444-8444-444444444444";

function project(overrides = {}) {
  return {
    legacy: false,
    owner: OWNER,
    projectChannelId: HOME,
    relatedChannelIds: [RELATED],
    repositories: [{ channelId: REPOSITORY }],
    ...overrides,
  };
}

function member(pubkey, role) {
  return { pubkey, role };
}

function channel(id, overrides = {}) {
  return {
    id,
    name: id,
    archivedAt: null,
    channelType: "stream",
    isMember: true,
    ...overrides,
  };
}

test("project owner and active home-channel admins can manage related channels", () => {
  assert.equal(
    canManageProjectRelatedChannels({
      homeChannelMembers: [],
      homeChannelActive: true,
      identityPubkey: OWNER.toUpperCase(),
      project: project(),
    }),
    true,
  );
  for (const role of ["owner", "admin"]) {
    assert.equal(
      canManageProjectRelatedChannels({
        homeChannelMembers: [member(ADMIN, role)],
        homeChannelActive: true,
        identityPubkey: ADMIN,
        project: project(),
      }),
      true,
    );
  }
});

test("ordinary members, legacy projects, and non-owners without a home channel cannot manage", () => {
  for (const role of ["member", "guest", "bot"]) {
    assert.equal(
      canManageProjectRelatedChannels({
        homeChannelMembers: [member(ADMIN, role)],
        homeChannelActive: true,
        identityPubkey: ADMIN,
        project: project(),
      }),
      false,
    );
  }
  assert.equal(
    canManageProjectRelatedChannels({
      homeChannelMembers: [member(ADMIN, "admin")],
      homeChannelActive: true,
      identityPubkey: ADMIN,
      project: project({ legacy: true }),
    }),
    false,
  );
  assert.equal(
    canManageProjectRelatedChannels({
      homeChannelMembers: [member(ADMIN, "admin")],
      homeChannelActive: true,
      identityPubkey: ADMIN,
      project: project({ projectChannelId: null }),
    }),
    false,
  );
  assert.equal(
    canManageProjectRelatedChannels({
      homeChannelActive: false,
      homeChannelMembers: [member(ADMIN, "admin")],
      identityPubkey: ADMIN,
      project: project(),
    }),
    false,
  );
});

test("link candidates exclude bound, unavailable, archived, and DM channels", () => {
  const candidates = listLinkableProjectChannels(project(), [
    channel(HOME),
    channel(RELATED),
    channel(REPOSITORY),
    channel(AVAILABLE, { name: "Available" }),
    channel("not-member", { isMember: false }),
    channel("archived", { archivedAt: "2026-09-01T00:00:00Z" }),
    channel("dm", { channelType: "dm" }),
  ]);
  assert.deepEqual(
    candidates.map((candidate) => candidate.id),
    [AVAILABLE],
  );
});
