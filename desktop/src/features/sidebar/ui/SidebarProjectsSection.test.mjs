import assert from "node:assert/strict";
import { test } from "node:test";

import {
  listSidebarProjects,
  readSidebarProjectExpansion,
  selectedProjectRouteId,
  writeSidebarProjectExpansion,
} from "./listSidebarProjects.ts";

const OWNER = "a".repeat(64);
const VIEWER = "b".repeat(64);

test("project expansion persists independently per relay and viewer", () => {
  const values = new Map();
  const previousLocalStorage = globalThis.localStorage;
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, value),
    },
  });
  try {
    writeSidebarProjectExpansion(
      { "project:one": true, "project:two": false },
      "https://relay.example",
      VIEWER,
    );
    assert.deepEqual(
      readSidebarProjectExpansion("https://relay.example", VIEWER),
      { "project:one": true, "project:two": false },
    );
    assert.deepEqual(
      readSidebarProjectExpansion("https://other.example", VIEWER),
      {},
    );
    assert.deepEqual(
      readSidebarProjectExpansion("https://relay.example", OWNER),
      {},
    );
  } finally {
    if (previousLocalStorage === undefined) {
      delete globalThis.localStorage;
    } else {
      Object.defineProperty(globalThis, "localStorage", {
        configurable: true,
        value: previousLocalStorage,
      });
    }
  }
});

function makeProject(overrides = {}) {
  return {
    createdAt: 0,
    description: "",
    dtag: "sprout",
    id: `30621:${OWNER}:sprout`,
    name: "sprout",
    owner: OWNER,
    primaryRepositoryAddress: null,
    projectAddress: `30621:${OWNER}:sprout`,
    projectChannelId: null,
    repositories: [],
    repositoryAddresses: [],
    status: "open",
    ...overrides,
  };
}

test("selectedProjectRouteId reads the project id from the detail route", () => {
  assert.equal(selectedProjectRouteId("/projects"), undefined);
  assert.equal(selectedProjectRouteId("/projects/"), undefined);
  assert.equal(selectedProjectRouteId("/agents"), undefined);
  assert.equal(
    selectedProjectRouteId("/projects/30621:abc:sprout"),
    "30621:abc:sprout",
  );
  assert.equal(
    selectedProjectRouteId("/projects/30621%3Aabc%3Asprout"),
    "30621:abc:sprout",
  );
});

test("listSidebarProjects only includes explicitly added projects", () => {
  const owned = makeProject({
    dtag: "owned",
    id: `30621:${VIEWER}:owned`,
    name: "Owned",
    owner: VIEWER,
    projectAddress: `30621:${VIEWER}:owned`,
  });
  const contributed = makeProject({
    dtag: "contrib",
    id: `30621:${OWNER}:contrib`,
    name: "Contrib",
    projectAddress: `30621:${OWNER}:contrib`,
    repositories: [
      {
        channelId: null,
        cloneUrls: [],
        contributors: [VIEWER],
        createdAt: 0,
        defaultBranch: "main",
        dtag: "repo",
        id: `${OWNER}:repo`,
        name: "repo",
        owner: OWNER,
        repoAddress: `30617:${OWNER}:repo`,
      },
    ],
  });
  const other = makeProject({ name: "Other" });

  const listed = listSidebarProjects({
    addedProjectAddresses: new Set([
      contributed.projectAddress,
      other.projectAddress,
    ]),
    currentPubkey: VIEWER,
    filter: "added",
    projects: [other, contributed, owned],
    sort: "name",
  });
  assert.deepEqual(
    listed.map((project) => project.name),
    ["Contrib", "Other"],
  );
});

test("listSidebarProjects owned filter hides contributed projects", () => {
  const owned = makeProject({
    dtag: "owned",
    id: `30621:${VIEWER}:owned`,
    name: "Owned",
    owner: VIEWER,
    projectAddress: `30621:${VIEWER}:owned`,
  });
  const contributed = makeProject({
    dtag: "contrib",
    id: `30621:${OWNER}:contrib`,
    name: "Contrib",
    projectAddress: `30621:${OWNER}:contrib`,
    repositories: [
      {
        channelId: null,
        cloneUrls: [],
        contributors: [VIEWER],
        createdAt: 0,
        defaultBranch: "main",
        dtag: "repo",
        id: `${OWNER}:repo`,
        name: "repo",
        owner: OWNER,
        repoAddress: `30617:${OWNER}:repo`,
      },
    ],
  });

  const listed = listSidebarProjects({
    addedProjectAddresses: new Set([
      contributed.projectAddress,
      owned.projectAddress,
    ]),
    currentPubkey: VIEWER,
    filter: "owned",
    projects: [contributed, owned],
    sort: "name",
  });
  assert.deepEqual(
    listed.map((project) => project.name),
    ["Owned"],
  );
});

test("listSidebarProjects excludes repository-only legacy read models", () => {
  const legacy = makeProject({
    dtag: "legacy-repo",
    id: `30617:${VIEWER}:legacy-repo`,
    legacy: true,
    name: "Legacy repo",
    owner: VIEWER,
    projectAddress: `30617:${VIEWER}:legacy-repo`,
  });

  for (const filter of ["added", "owned"]) {
    const listed = listSidebarProjects({
      addedProjectAddresses: new Set([legacy.projectAddress]),
      currentPubkey: VIEWER,
      filter,
      projects: [legacy],
      sort: "name",
    });
    assert.deepEqual(listed, []);
  }
});

test("listSidebarProjects owned filter surfaces owned projects never added", () => {
  const owned = makeProject({
    dtag: "owned",
    id: `30621:${VIEWER}:owned`,
    name: "Owned",
    owner: VIEWER,
    projectAddress: `30621:${VIEWER}:owned`,
  });
  const other = makeProject({ name: "Other" });

  // The owned project is absent from the added set: "owned" must still
  // discover it, while "added" keeps listing only explicit membership.
  const addedProjectAddresses = new Set([other.projectAddress]);
  const ownedListing = listSidebarProjects({
    addedProjectAddresses,
    currentPubkey: VIEWER,
    filter: "owned",
    projects: [other, owned],
    sort: "name",
  });
  assert.deepEqual(
    ownedListing.map((project) => project.name),
    ["Owned"],
  );
  const addedListing = listSidebarProjects({
    addedProjectAddresses,
    currentPubkey: VIEWER,
    filter: "added",
    projects: [other, owned],
    sort: "name",
  });
  assert.deepEqual(
    addedListing.map((project) => project.name),
    ["Other"],
  );
});

test("listSidebarProjects newest sort orders by createdAt then name", () => {
  const older = makeProject({
    createdAt: 10,
    dtag: "older",
    id: `30621:${VIEWER}:older`,
    name: "Alpha",
    owner: VIEWER,
    projectAddress: `30621:${VIEWER}:older`,
  });
  const newer = makeProject({
    createdAt: 20,
    dtag: "newer",
    id: `30621:${VIEWER}:newer`,
    name: "Zebra",
    owner: VIEWER,
    projectAddress: `30621:${VIEWER}:newer`,
  });

  const listed = listSidebarProjects({
    addedProjectAddresses: new Set([
      older.projectAddress,
      newer.projectAddress,
    ]),
    currentPubkey: VIEWER,
    filter: "added",
    projects: [older, newer],
    sort: "created",
  });
  assert.deepEqual(
    listed.map((project) => project.name),
    ["Zebra", "Alpha"],
  );
});
