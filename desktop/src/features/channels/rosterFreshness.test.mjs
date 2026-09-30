// A roster refresh after a known membership change must read from the relay
// writer: the replica can still return the pre-change roster, which would
// then stay fresh for the whole roster freshness window.
import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryObserver } from "@tanstack/react-query";

const memberCalls = [];
const pendingCalls = [];
globalThis.window = {
  setTimeout,
  clearTimeout,
  __TAURI_INTERNALS__: {
    invoke: (command, args) => {
      if (command !== "get_channel_members") return Promise.resolve(undefined);
      memberCalls.push(args);
      return new Promise((resolve) => pendingCalls.push({ args, resolve }));
    },
  },
};

const {
  channelMembersQueryKey,
  fetchChannelMembers,
  invalidateChannelMembersRosters,
} = await import("./rosterFreshness.ts");

const roster = (...pubkeys) => ({
  members: pubkeys.map((pubkey) => ({ pubkey, role: "member" })),
});

function observeRoster(queryClient, channelId) {
  const observer = new QueryObserver(queryClient, {
    queryKey: channelMembersQueryKey(channelId),
    queryFn: ({ signal }) => fetchChannelMembers(channelId, signal),
    staleTime: 5 * 60_000,
  });
  const unsubscribe = observer.subscribe(() => {});
  return { observer, unsubscribe };
}

const rosterPubkeys = (queryClient, channelId) =>
  queryClient
    .getQueryData(channelMembersQueryKey(channelId))
    ?.map((member) => member.pubkey);

async function settle() {
  for (let i = 0; i < 5; i += 1) await new Promise((r) => setTimeout(r, 0));
}

test("a membership-change refresh reads from the writer and beats an in-flight replica read", async () => {
  memberCalls.length = 0;
  pendingCalls.length = 0;
  const queryClient = new QueryClient();
  // Mounting the roster starts an ordinary display fetch on the replica.
  const { unsubscribe } = observeRoster(queryClient, "ch-a");
  await settle();
  assert.equal(memberCalls.length, 1);
  assert.equal(memberCalls[0].readYourWrites, undefined);

  // The app adds a member while that replica read is still in flight.
  const refresh = invalidateChannelMembersRosters(queryClient, ["ch-a"]);
  await settle();
  assert.equal(memberCalls.length, 2);
  assert.equal(memberCalls[1].readYourWrites, true);

  // The writer read lands first; the lagging replica read resolves late.
  pendingCalls[1].resolve(roster("owner", "bot"));
  await refresh;
  pendingCalls[0].resolve(roster("owner"));
  await settle();
  assert.deepEqual(rosterPubkeys(queryClient, "ch-a"), ["owner", "bot"]);

  // One completed writer read clears the mark; display reads use the replica.
  const next = queryClient.refetchQueries({
    queryKey: channelMembersQueryKey("ch-a"),
  });
  await settle();
  assert.equal(memberCalls.at(-1).readYourWrites, undefined);
  pendingCalls.at(-1).resolve(roster("owner", "bot"));
  await next;
  unsubscribe();
});

test("a writer read cancelled by a broad refetch keeps the replacement on the writer", async () => {
  memberCalls.length = 0;
  pendingCalls.length = 0;
  const queryClient = new QueryClient();
  const { unsubscribe } = observeRoster(queryClient, "ch-b");
  await settle();
  pendingCalls[0].resolve(roster("owner"));
  await settle();

  const refresh = invalidateChannelMembersRosters(queryClient, ["ch-b"]);
  await settle();
  assert.equal(memberCalls.at(-1).readYourWrites, true);

  // Invalidating ["channels"] cancels the in-flight writer read and refetches.
  const broad = queryClient.invalidateQueries({ queryKey: ["channels"] });
  await settle();
  assert.equal(memberCalls.length, 3);
  assert.equal(memberCalls[2].readYourWrites, true);

  // The cancelled read finishing must not clear the mark: a second broad
  // refetch that cancels the replacement still has to read from the writer.
  pendingCalls[1].resolve(roster("owner"));
  await settle();
  const broadAgain = queryClient.invalidateQueries({ queryKey: ["channels"] });
  await settle();
  assert.equal(memberCalls.length, 4);
  assert.equal(memberCalls[3].readYourWrites, true);
  pendingCalls[2].resolve(roster("owner"));
  pendingCalls[3].resolve(roster("owner", "bot"));
  await Promise.all([refresh, broad, broadAgain]);
  assert.deepEqual(rosterPubkeys(queryClient, "ch-b"), ["owner", "bot"]);
  unsubscribe();
});
