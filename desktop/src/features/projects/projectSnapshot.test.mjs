import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient } from "@tanstack/react-query";
import {
  isProjectCollectionAuthoritative,
  isProjectDataAuthoritative,
  isProjectRelayValidated,
  markProjectCollectionAuthoritative,
  markProjectDataAuthoritative,
  persistProjectSnapshot,
  PROJECT_QUERY_STRUCTURAL_SHARING,
  projectSnapshotKey,
  readProjectSnapshot,
  removeProjectSnapshotForRelay,
  seedProjectSnapshot,
  shouldUseScopedProjectHomeLookup,
} from "./projectSnapshot.ts";

if (typeof globalThis.window === "undefined") {
  const storage = new Map();
  globalThis.window = {
    localStorage: {
      get length() {
        return storage.size;
      },
      key: (index) => [...storage.keys()][index] ?? null,
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
    },
  };
}

const RELAY = "wss://relay.example.com";
const OWNER = "a".repeat(64);
const PROJECT = {
  id: `30621:${OWNER}:relay`,
  dtag: "relay",
  name: "Relay",
  description: "",
  owner: OWNER,
  createdAt: 100,
  projectChannelId: "11111111-1111-4111-8111-111111111111",
  relatedChannelIds: [],
  status: "active",
  projectAddress: `30621:${OWNER}:relay`,
  primaryRepositoryAddress: null,
  repositoryAddresses: [],
  repositoryRelayHints: {},
  repositories: [],
  unavailableRepositoryAddresses: [],
  visibility: "listed",
  legacy: false,
};

test.beforeEach(() => {
  removeProjectSnapshotForRelay(RELAY);
});

test("project snapshot is scoped by normalized relay and identity", () => {
  const client = new QueryClient();
  seedProjectSnapshot(client, { pubkey: OWNER, relayUrl: RELAY });
  persistProjectSnapshot(client, [PROJECT]);

  assert.deepEqual(readProjectSnapshot(RELAY, OWNER), [PROJECT]);
  assert.equal(readProjectSnapshot(RELAY, "b".repeat(64)), null);
  assert.equal(
    projectSnapshotKey("WSS://Relay.Example.com/", OWNER.toUpperCase()),
    projectSnapshotKey(RELAY, OWNER),
  );
});

test("seedProjectSnapshot paints stale data into a fresh query client", () => {
  const writer = new QueryClient();
  seedProjectSnapshot(writer, { pubkey: OWNER, relayUrl: RELAY });
  persistProjectSnapshot(writer, [PROJECT]);

  const reader = new QueryClient();
  seedProjectSnapshot(reader, { pubkey: OWNER, relayUrl: RELAY });

  assert.deepEqual(reader.getQueryData(["projects"]), [PROJECT]);
  assert.equal(reader.getQueryState(["projects"])?.dataUpdatedAt, 0);
  const snapshotProject = reader.getQueryData(["projects"])[0];
  assert.equal(isProjectDataAuthoritative(snapshotProject), false);
  assert.equal(isProjectCollectionAuthoritative(reader), false);

  const locallyWrittenProject = markProjectDataAuthoritative(
    { ...PROJECT, id: `${PROJECT.id}:local` },
    "local-write",
  );
  reader.setQueryData(["projects"], (current = []) => [
    ...current,
    locallyWrittenProject,
  ]);

  assert.ok((reader.getQueryState(["projects"])?.dataUpdatedAt ?? 0) > 0);
  assert.equal(isProjectCollectionAuthoritative(reader), false);
  assert.equal(isProjectDataAuthoritative(snapshotProject), false);
  assert.equal(isProjectDataAuthoritative(locallyWrittenProject), true);
  assert.equal(isProjectRelayValidated(locallyWrittenProject), false);
  assert.equal(
    shouldUseScopedProjectHomeLookup({
      collectionIsAuthoritative: isProjectCollectionAuthoritative(reader),
      hasEnumeratedProjectHome: false,
      isHuddleTranscript: false,
    }),
    true,
  );
});

test("successful relay data is authoritative", () => {
  const client = new QueryClient();
  const relayProject = markProjectDataAuthoritative({ ...PROJECT }, "relay");
  client.setQueryData(["projects"], [relayProject]);
  markProjectCollectionAuthoritative(client);

  assert.equal(isProjectDataAuthoritative(relayProject), true);
  assert.equal(isProjectRelayValidated(relayProject), true);
  assert.equal(isProjectCollectionAuthoritative(client), true);
  assert.equal(
    shouldUseScopedProjectHomeLookup({
      collectionIsAuthoritative: isProjectCollectionAuthoritative(client),
      hasEnumeratedProjectHome: false,
      isHuddleTranscript: false,
    }),
    false,
  );
});

test("equal relay data replaces snapshot objects and preserves provenance", async () => {
  const writer = new QueryClient();
  seedProjectSnapshot(writer, { pubkey: OWNER, relayUrl: RELAY });
  persistProjectSnapshot(writer, [PROJECT]);

  const reader = new QueryClient();
  seedProjectSnapshot(reader, { pubkey: OWNER, relayUrl: RELAY });
  const snapshotProject = reader.getQueryData(["projects"])[0];
  const liveProject = markProjectDataAuthoritative({ ...PROJECT }, "relay");

  await reader.fetchQuery({
    queryKey: ["projects"],
    queryFn: async () => {
      markProjectCollectionAuthoritative(reader);
      return [liveProject];
    },
    structuralSharing: PROJECT_QUERY_STRUCTURAL_SHARING,
  });

  const cachedProject = reader.getQueryData(["projects"])[0];
  assert.notEqual(cachedProject, snapshotProject);
  assert.equal(cachedProject, liveProject);
  assert.equal(isProjectRelayValidated(cachedProject), true);
  assert.equal(isProjectCollectionAuthoritative(reader), true);
});
