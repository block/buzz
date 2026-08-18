import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryObserver } from "@tanstack/react-query";

import { updateCachedChannelMemberDisplayName } from "./channelMemberProfileCache.ts";

function member(pubkey, displayName, overrides = {}) {
  return {
    pubkey,
    role: "bot",
    isAgent: true,
    joinedAt: null,
    displayName,
    ...overrides,
  };
}

test("updateCachedChannelMemberDisplayName_updatesOnlyMatchingMemberQueries", async () => {
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  const unrelatedMembersKey = ["channels", "random", "members"];
  const detailKey = ["channels", "general", "detail"];

  queryClient.setQueryData(matchingKey, [
    member("AABB", "Old name"),
    member("ccdd", "Someone else", { role: "member", isAgent: false }),
  ]);
  queryClient.setQueryData(unrelatedMembersKey, [member("eeff", "Other")]);
  queryClient.setQueryData(detailKey, { name: "General" });

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");

  assert.deepEqual(queryClient.getQueryData(matchingKey), [
    member("AABB", "New name"),
    member("ccdd", "Someone else", { role: "member", isAgent: false }),
  ]);
  assert.deepEqual(queryClient.getQueryData(unrelatedMembersKey), [
    member("eeff", "Other"),
  ]);
  assert.deepEqual(queryClient.getQueryData(detailKey), { name: "General" });
  assert.equal(queryClient.getQueryState(matchingKey)?.isInvalidated, true);
  assert.equal(
    queryClient.getQueryState(unrelatedMembersKey)?.isInvalidated,
    false,
  );
  assert.equal(queryClient.getQueryState(detailKey)?.isInvalidated, false);
});

test("updateCachedChannelMemberDisplayName_doesNotRefetchMatchingQueries", async () => {
  let fetchCount = 0;
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        queryFn: async () => {
          fetchCount += 1;
          return [member("aabb", "Relay name")];
        },
      },
    },
  });
  const matchingKey = ["channels", "general", "members"];
  queryClient.setQueryData(matchingKey, [member("aabb", "Old name")]);

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");

  assert.equal(fetchCount, 0);
  assert.equal(
    queryClient.getQueryData(matchingKey)[0].displayName,
    "New name",
  );
});

test("updateCachedChannelMemberDisplayName_cancelsStaleInFlightFetches", async () => {
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  let resolveFetch;

  queryClient.setQueryData(matchingKey, [member("aabb", "Old name")]);
  const fetch = queryClient.fetchQuery({
    queryKey: matchingKey,
    queryFn: () =>
      new Promise((resolve) => {
        resolveFetch = resolve;
      }),
  });

  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");
  resolveFetch([member("aabb", "Stale relay name")]);
  await fetch.catch(() => undefined);

  assert.equal(
    queryClient.getQueryData(matchingKey)[0].displayName,
    "New name",
  );
});

test("updateCachedChannelMemberDisplayName_restartsFirstLoadMemberFetches", async () => {
  const queryClient = new QueryClient();
  const matchingKey = ["channels", "general", "members"];
  const fetchResolvers = [];
  const observer = new QueryObserver(queryClient, {
    queryKey: matchingKey,
    queryFn: () =>
      new Promise((resolve) => {
        fetchResolvers.push(resolve);
      }),
  });
  let resolveObservedData;
  const observedData = new Promise((resolve) => {
    resolveObservedData = resolve;
  });
  const unsubscribe = observer.subscribe((result) => {
    if (result.data) {
      resolveObservedData();
    }
  });

  assert.equal(fetchResolvers.length, 1);
  await updateCachedChannelMemberDisplayName(queryClient, "aabb", "New name");

  assert.equal(fetchResolvers.length, 2);
  fetchResolvers[1]([member("aabb", "New name")]);
  fetchResolvers[0]([member("aabb", "Stale relay name")]);
  await observedData;

  assert.equal(
    queryClient.getQueryData(matchingKey)[0].displayName,
    "New name",
  );
  unsubscribe();
});
