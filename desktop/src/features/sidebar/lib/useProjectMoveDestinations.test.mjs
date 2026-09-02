import assert from "node:assert/strict";
import test from "node:test";

import {
  listMemberProjectMoveDestinations,
  moveChannelToProjectSection,
} from "./useProjectMoveDestinations.ts";

const OWNER = "a".repeat(64);

function makeProject(overrides = {}) {
  return {
    id: `30621:${OWNER}:alpha`,
    dtag: "alpha",
    name: "Alpha",
    description: "",
    owner: OWNER,
    createdAt: 100,
    projectChannelId: "home-alpha",
    relatedChannelIds: [],
    status: "active",
    projectAddress: `30621:${OWNER}:alpha`,
    primaryRepositoryAddress: null,
    repositoryAddresses: [],
    repositories: [],
    legacy: false,
    ...overrides,
  };
}

function makeChannel(id, isMember) {
  return {
    id,
    name: id,
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 1,
    memberPubkeys: [],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

test("lists only projects whose home channels include the viewer", () => {
  const projectSection = {
    id: "section-alpha",
    name: "Alpha",
    order: 0,
  };
  const destinations = listMemberProjectMoveDestinations({
    channels: [
      makeChannel("home-alpha", true),
      makeChannel("home-beta", false),
    ],
    projects: [
      makeProject(),
      makeProject({
        id: `30621:${OWNER}:beta`,
        dtag: "beta",
        name: "Beta",
        projectAddress: `30621:${OWNER}:beta`,
        projectChannelId: "home-beta",
      }),
      makeProject({
        id: "legacy",
        legacy: true,
        name: "Legacy",
        projectAddress: "legacy",
      }),
    ],
    readPreferences: (channelId) =>
      channelId === "home-alpha"
        ? { breakoutSectionId: projectSection.id }
        : {},
    sections: [projectSection],
  });

  assert.deepEqual(destinations, [
    {
      name: "Alpha",
      projectAddress: `30621:${OWNER}:alpha`,
      projectChannelId: "home-alpha",
      relatedChannelIds: [],
      sectionId: "section-alpha",
    },
  ]);
});

test("ignores a stale project section preference", () => {
  const [destination] = listMemberProjectMoveDestinations({
    channels: [makeChannel("home-alpha", true)],
    projects: [makeProject()],
    readPreferences: () => ({ breakoutSectionId: "deleted-section" }),
    sections: [],
  });

  assert.equal(destination.sectionId, null);
});

test("moving to a project creates one section and groups its channels", () => {
  const assignments = [];
  const destination = {
    name: "Alpha",
    projectAddress: `30621:${OWNER}:alpha`,
    projectChannelId: "home-alpha",
    relatedChannelIds: ["related", "target"],
    sectionId: null,
  };
  let createCalls = 0;
  const section = moveChannelToProjectSection({
    assignChannel: (channelId, sectionId) =>
      assignments.push([channelId, sectionId]),
    channelId: "target",
    createSection: (name) => {
      createCalls += 1;
      return { id: "new-section", name, order: 0 };
    },
    destination,
    sections: [],
  });

  assert.equal(section?.id, "new-section");
  assert.equal(createCalls, 1);
  assert.deepEqual(assignments, [
    ["home-alpha", "new-section"],
    ["related", "new-section"],
    ["target", "new-section"],
  ]);
});

test("moving to a project reuses its existing section", () => {
  const existing = { id: "section-alpha", name: "Alpha", order: 0 };
  let createCalls = 0;
  const assignments = [];
  const section = moveChannelToProjectSection({
    assignChannel: (channelId, sectionId) =>
      assignments.push([channelId, sectionId]),
    channelId: "target",
    createSection: () => {
      createCalls += 1;
      return null;
    },
    destination: {
      name: "Alpha",
      projectAddress: `30621:${OWNER}:alpha`,
      projectChannelId: "home-alpha",
      relatedChannelIds: [],
      sectionId: existing.id,
    },
    sections: [existing],
  });

  assert.equal(section, existing);
  assert.equal(createCalls, 0);
  assert.deepEqual(assignments, [
    ["home-alpha", "section-alpha"],
    ["target", "section-alpha"],
  ]);
});
