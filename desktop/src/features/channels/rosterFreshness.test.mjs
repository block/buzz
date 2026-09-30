// After this client changes a channel's membership, the member list shown
// must never settle on the list from before the change, even though the
// relay's replica keeps returning that stale list for a while.
import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryObserver } from "@tanstack/react-query";

let now = 1_000_000;
Date.now = () => now;

const OWNER = "aa".repeat(32);
const BOT = "bb".repeat(32);
const routes = [];
let writerMembers = [OWNER];
const replicaMembers = [OWNER];
const deferred = [];
let deferNextRead = false;

globalThis.window = {
  setTimeout,
  clearTimeout,
  __TAURI_INTERNALS__: {
    invoke: async (command, args) => {
      if (command === "get_channel_members") {
        const writer = args.readYourWrites === true;
        routes.push(writer ? "writer" : "replica");
        const members = writer ? writerMembers : replicaMembers;
        const response = {
          members: members.map((pubkey) => ({ pubkey, role: "member" })),
        };
        if (deferNextRead) {
          deferNextRead = false;
          return new Promise((resolve) =>
            deferred.push(() => resolve(response)),
          );
        }
        return response;
      }
      if (command === "add_channel_members") {
        writerMembers = [...writerMembers, ...args.pubkeys];
        return { added: args.pubkeys, errors: [] };
      }
      if (command === "join_channel") {
        writerMembers = [...writerMembers, BOT];
        return undefined;
      }
      return undefined;
    },
  },
};

const { addChannelMembers, joinChannel, getChannelMembers } = await import(
  "@/shared/api/tauri"
);
const { channelsQueryKey, invalidateChannelState } = await import("./hooks.ts");
const { channelMembersQueryKey, refreshRostersOnMembershipChange } =
  await import("./rosterFreshness.ts");

function setup(channelId) {
  routes.length = 0;
  writerMembers = [OWNER];
  now += 60_000;
  const queryClient = new QueryClient();
  const unsubscribeRefresh = refreshRostersOnMembershipChange(queryClient);
  // Same query function as useChannelMembersQuery.
  const observer = new QueryObserver(queryClient, {
    queryKey: channelMembersQueryKey(channelId),
    queryFn: () => getChannelMembers(channelId),
    staleTime: 5 * 60_000,
  });
  return {
    queryClient,
    observer,
    mount: () => observer.subscribe(() => {}),
    shown: () =>
      queryClient
        .getQueryData(channelMembersQueryKey(channelId))
        ?.map((member) => member.pubkey),
    teardown: unsubscribeRefresh,
  };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

test("adding a member then invalidating channel state keeps the new roster", async () => {
  const roster = setup("ch-add");
  const unmount = roster.mount();
  await settle();
  assert.deepEqual(roster.shown(), [OWNER]);

  await addChannelMembers({ channelId: "ch-add", pubkeys: [BOT], role: "bot" });
  await invalidateChannelState(roster.queryClient, "ch-add");
  await settle();

  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  assert.deepEqual(
    routes.slice(1),
    routes.slice(1).map(() => "writer"),
  );
  assert.ok(routes.length > 1);
  unmount();
  roster.teardown();
});

test("joining from the browser then opening the channel shows the joined roster", async () => {
  const roster = setup("ch-join");
  // AppShell's browser join handler, then the dialog opens the channel.
  await joinChannel("ch-join");
  await roster.queryClient.invalidateQueries({ queryKey: channelsQueryKey });
  const unmount = roster.mount();
  await settle();

  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  assert.deepEqual(routes, ["writer"]);
  unmount();
  roster.teardown();
});

test("a replica read in flight before the change cannot land over it", async () => {
  const roster = setup("ch-race");
  deferNextRead = true;
  const unmount = roster.mount();
  await settle();
  assert.deepEqual(routes, ["replica"]);

  await addChannelMembers({
    channelId: "ch-race",
    pubkeys: [BOT],
    role: "bot",
  });
  await settle();
  for (const resolve of deferred.splice(0)) resolve();
  await settle();

  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  unmount();
  roster.teardown();
});

test("ordinary reads return to the replica once the window passes", async () => {
  const roster = setup("ch-later");
  await addChannelMembers({
    channelId: "ch-later",
    pubkeys: [BOT],
    role: "bot",
  });
  now += 5_001;
  await getChannelMembers("ch-later");
  await getChannelMembers("ch-other");
  assert.deepEqual(routes.slice(-2), ["replica", "replica"]);
  roster.teardown();
});
