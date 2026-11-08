import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient } from "@tanstack/react-query";

import {
  applyUserStatusEventToQueries,
  expireUserStatusQueries,
  userStatusQueryKey,
  visibleUserStatus,
} from "./hooks.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);

function status(expiresAt) {
  return {
    text: "Busy",
    emoji: "🔴",
    updatedAt: 1,
    eventId: "event-1",
    expiresAt,
  };
}

test("expires due statuses in every cached lookup without relay traffic", () => {
  const queryClient = new QueryClient();
  queryClient.setQueryData(userStatusQueryKey([ALICE]), {
    [ALICE]: status(100),
  });
  queryClient.setQueryData(userStatusQueryKey([ALICE, BOB]), {
    [ALICE]: status(100),
    [BOB]: status(101),
  });

  assert.equal(expireUserStatusQueries(queryClient, 99), false);
  assert.equal(expireUserStatusQueries(queryClient, 100), true);
  assert.deepEqual(queryClient.getQueryData(userStatusQueryKey([ALICE])), {
    [ALICE]: {
      text: "",
      emoji: "",
      updatedAt: 1,
      eventId: "event-1",
    },
  });
  assert.deepEqual(queryClient.getQueryData(userStatusQueryKey([ALICE, BOB])), {
    [ALICE]: {
      text: "",
      emoji: "",
      updatedAt: 1,
      eventId: "event-1",
    },
    [BOB]: status(101),
  });
});

function statusEvent({ id, createdAt, text = "Busy", expiresAt }) {
  const tags = [
    ["d", "general"],
    ["emoji", "🔴"],
  ];
  if (expiresAt !== undefined) tags.push(["expiration", String(expiresAt)]);
  return {
    id,
    kind: 30315,
    pubkey: ALICE,
    content: text,
    tags,
    created_at: createdAt,
    sig: "",
  };
}

test("applies a live status to a requested lookup before history resolves", () => {
  const queryClient = new QueryClient();
  const queryKey = userStatusQueryKey([ALICE]);
  queryClient.setQueryData(queryKey, {});

  applyUserStatusEventToQueries(
    queryClient,
    statusEvent({ id: "live", createdAt: 101 }),
    101,
  );

  assert.equal(
    visibleUserStatus(queryClient.getQueryData(queryKey)?.[ALICE])?.text,
    "Busy",
  );
});

test("expiration retains the replacement fence against delayed older events", () => {
  const queryClient = new QueryClient();
  const queryKey = userStatusQueryKey([ALICE]);
  queryClient.setQueryData(queryKey, { [ALICE]: null });

  applyUserStatusEventToQueries(
    queryClient,
    statusEvent({ id: "b", createdAt: 101, expiresAt: 102 }),
    101,
  );
  assert.equal(
    visibleUserStatus(queryClient.getQueryData(queryKey)[ALICE])?.text,
    "Busy",
  );

  assert.equal(expireUserStatusQueries(queryClient, 102), true);
  assert.equal(
    visibleUserStatus(queryClient.getQueryData(queryKey)[ALICE]),
    null,
  );

  applyUserStatusEventToQueries(
    queryClient,
    statusEvent({ id: "a", createdAt: 100, text: "Older" }),
    102,
  );
  assert.equal(
    visibleUserStatus(queryClient.getQueryData(queryKey)[ALICE]),
    null,
  );

  applyUserStatusEventToQueries(
    queryClient,
    statusEvent({ id: "c", createdAt: 103, text: "Newer" }),
    102,
  );
  assert.equal(
    visibleUserStatus(queryClient.getQueryData(queryKey)[ALICE])?.text,
    "Newer",
  );
});

test("an ordinary cache update settles without an expiration write loop", () => {
  const queryClient = new QueryClient();
  let statusUpdates = 0;
  const unsubscribe = queryClient.getQueryCache().subscribe((event) => {
    if (event.type !== "updated" || event.query.queryKey[0] !== "user-status") {
      return;
    }
    statusUpdates += 1;
    if (statusUpdates < 5) {
      expireUserStatusQueries(queryClient, 99);
    }
  });

  queryClient.setQueryData(userStatusQueryKey([ALICE]), {
    [ALICE]: status(100),
  });
  unsubscribe();

  assert.equal(statusUpdates, 1);
});
