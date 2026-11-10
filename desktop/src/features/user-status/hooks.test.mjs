import assert from "node:assert/strict";
import test from "node:test";
import { QueryClient } from "@tanstack/react-query";

import {
  fetchUserStatusLookup,
  readCurrentUserStatusLookup,
  USER_STATUS_AUTHOR_CHUNK_SIZE,
} from "./hooks.ts";

function statusEvent(pubkey, createdAt = 1) {
  return {
    id: `${pubkey}-${createdAt}`,
    kind: 30315,
    pubkey,
    content: `Status ${pubkey}`,
    tags: [
      ["d", "general"],
      ["emoji", "💬"],
    ],
    created_at: createdAt,
    sig: "",
  };
}

test("does not let a pending history result replace a newer live status", async () => {
  const pubkey = "a".repeat(64);
  let resolveHistory;
  let current = {};
  const lookupPromise = fetchUserStatusLookup(
    [pubkey],
    () => new Promise((resolve) => (resolveHistory = resolve)),
    () => current,
  );
  await new Promise((resolve) => setImmediate(resolve));
  current = {
    [pubkey]: {
      text: "Live",
      emoji: "💬",
      updatedAt: 2,
      eventId: "live",
    },
  };
  resolveHistory([statusEvent(pubkey, 1)]);

  const lookup = await lookupPromise;

  assert.equal(lookup[pubkey]?.text, "Live");
  assert.equal(lookup[pubkey]?.eventId, "live");
});

test("seeds overlapping status lookups with the newest cached version", () => {
  const pubkey = "a".repeat(64);
  const queryClient = new QueryClient();
  queryClient.setQueryData(["user-status", pubkey, "newer"], {
    [pubkey]: {
      text: "Newer",
      emoji: "",
      updatedAt: 2,
      eventId: "newer",
    },
  });
  queryClient.setQueryData(["user-status", pubkey, "older"], {
    [pubkey]: {
      text: "Older",
      emoji: "",
      updatedAt: 1,
      eventId: "older",
    },
  });

  const lookup = readCurrentUserStatusLookup(queryClient, [pubkey]);

  assert.equal(lookup[pubkey]?.text, "Newer");
  assert.equal(lookup[pubkey]?.eventId, "newer");
  queryClient.clear();
});

test("fetches every current status when the author set exceeds one relay page", async () => {
  const pubkeys = Array.from(
    { length: USER_STATUS_AUTHOR_CHUNK_SIZE + 7 },
    (_, index) => index.toString(16).padStart(64, "0"),
  );
  const filters = [];
  const lookup = await fetchUserStatusLookup(pubkeys, async (filter) => {
    filters.push(filter);
    return filter.authors.map((pubkey) => statusEvent(pubkey));
  });

  assert.equal(filters.length, 2);
  assert.equal(filters[0].authors.length, USER_STATUS_AUTHOR_CHUNK_SIZE);
  assert.equal(filters[1].authors.length, 7);
  assert.equal(Object.keys(lookup).length, pubkeys.length);
  for (const pubkey of pubkeys) {
    assert.equal(lookup[pubkey]?.text, `Status ${pubkey}`);
  }
});
