import assert from "node:assert/strict";
import test from "node:test";

import {
  mergeProjectSidebarMembershipStores,
  parseProjectSidebarMembershipPayload,
  selectedProjectAddressesFromStore,
} from "./projectSidebarMembership.ts";

function store(projects = {}) {
  return { version: 1, projects };
}

test("migrates the legacy selected-address array", () => {
  const parsed = parseProjectSidebarMembershipPayload([
    "30621:alice:buzz",
    "30621:alice:buzz",
    "",
    42,
  ]);

  assert.deepEqual(parsed, {
    version: 1,
    projects: {
      "30621:alice:buzz": { selected: true, updatedAt: 0 },
    },
  });
});

test("merge preserves independent selections from two clients", () => {
  const merged = mergeProjectSidebarMembershipStores(
    store({
      "30621:alice:one": { selected: true, updatedAt: 100 },
    }),
    store({
      "30621:alice:two": { selected: true, updatedAt: 200 },
    }),
  );

  assert.deepEqual(selectedProjectAddressesFromStore(merged).sort(), [
    "30621:alice:one",
    "30621:alice:two",
  ]);
});

test("newer removal wins over an older selection", () => {
  const merged = mergeProjectSidebarMembershipStores(
    store({
      "30621:alice:buzz": { selected: true, updatedAt: 100 },
    }),
    store({
      "30621:alice:buzz": { selected: false, updatedAt: 200 },
    }),
  );

  assert.deepEqual(selectedProjectAddressesFromStore(merged), []);
});

test("equal-timestamp conflicts converge with removal winning", () => {
  const selected = store({
    "30621:alice:buzz": { selected: true, updatedAt: 100 },
  });
  const removed = store({
    "30621:alice:buzz": { selected: false, updatedAt: 100 },
  });

  assert.deepEqual(
    mergeProjectSidebarMembershipStores(selected, removed),
    mergeProjectSidebarMembershipStores(removed, selected),
  );
  assert.deepEqual(
    selectedProjectAddressesFromStore(
      mergeProjectSidebarMembershipStores(selected, removed),
    ),
    [],
  );
});

test("drops malformed entries and rejects unknown versions", () => {
  assert.deepEqual(
    parseProjectSidebarMembershipPayload({
      version: 1,
      projects: { project: { selected: "yes", updatedAt: 1 } },
    }),
    store(),
  );
  assert.equal(
    parseProjectSidebarMembershipPayload({ version: 2, projects: {} }),
    null,
  );
});
