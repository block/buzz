// After this client changes a channel's membership, the member list shown
// must never settle on the list from before the change, even though the
// relay's replica keeps returning that stale list for a while.
import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryObserver } from "@tanstack/react-query";

let now = 1_000_000;
// The window runs on the monotonic clock; wall-clock jumps must not move it.
Object.defineProperty(performance, "now", { value: () => now });

const OWNER = "aa".repeat(32);
const BOT = "bb".repeat(32);
const routes = [];
let writerMembers = [OWNER];
let replicaMembers = [OWNER];
let nativeResults = {};
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
      if (command in nativeResults) {
        writerMembers = [...writerMembers, BOT];
        return nativeResults[command];
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
const { ensureStarterChannels, syncAgentsToActiveHuddle } = await import(
  "@/shared/api/tauriChannels"
);
const { resolveBestieConversation } = await import(
  "@/protectedFeatures/bestie/api"
);
const { noteChannelMembershipChange, resetChannelMembershipWrites } =
  await import("@/shared/api/channelMembershipWrites");
const { channelsQueryKey, invalidateChannelState } = await import("./hooks.ts");
const { channelMembersQueryKey, refreshRostersOnMembershipChange } =
  await import("./rosterFreshness.ts");

function setup(channelId) {
  routes.length = 0;
  writerMembers = [OWNER];
  replicaMembers = [OWNER];
  nativeResults = {};
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

test("the window outlasts the relay's 30s staleness ceiling", async () => {
  const roster = setup("ch-ceiling");
  const unmount = roster.mount();
  await settle();
  await addChannelMembers({
    channelId: "ch-ceiling",
    pubkeys: [BOT],
    role: "bot",
  });
  await settle();
  // A 30s replica budget may still legally serve the old list here.
  now += 30_000;
  await roster.queryClient.invalidateQueries({
    queryKey: channelMembersQueryKey("ch-ceiling"),
  });
  await settle();
  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  assert.equal(routes.at(-1), "writer");

  // Past the ceiling the replica has caught up, and reads return to it.
  now += 1_001;
  replicaMembers = [OWNER, BOT];
  await roster.queryClient.invalidateQueries({
    queryKey: channelMembersQueryKey("ch-ceiling"),
  });
  await settle();
  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  assert.equal(routes.at(-1), "replica");
  unmount();
  roster.teardown();
});

test("a wall-clock jump does not end the window early", async () => {
  const roster = setup("ch-clock");
  await addChannelMembers({
    channelId: "ch-clock",
    pubkeys: [BOT],
    role: "bot",
  });
  const realDateNow = Date.now;
  Date.now = () => realDateNow() + 3_600_000;
  try {
    await getChannelMembers("ch-clock");
  } finally {
    Date.now = realDateNow;
  }
  assert.equal(routes.at(-1), "writer");
  roster.teardown();
});

async function cacheOldRoster(roster) {
  const unmount = roster.mount();
  await settle();
  assert.deepEqual(roster.shown(), [OWNER]);
  return unmount;
}

test("starter joins that landed are recorded when a later join fails", async () => {
  const roster = setup("starter-general");
  const unmount = await cacheOldRoster(roster);
  nativeResults.ensure_starter_channels = {
    channels: [],
    changed_channel_ids: ["starter-general"],
    error: "starter join rejected",
  };
  await assert.rejects(ensureStarterChannels(), {
    message: "starter join rejected",
  });
  await settle();
  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  unmount();
  roster.teardown();
});

test("huddle sync records the parent channel it changed", async () => {
  const roster = setup("huddle-parent");
  const unmount = await cacheOldRoster(roster);
  nativeResults.sync_agents_to_active_huddle = {
    matched_active_huddle: true,
    added: [BOT],
    changed_channel_ids: ["huddle-ephemeral", "huddle-parent"],
    error: null,
  };
  await syncAgentsToActiveHuddle("huddle-ephemeral", [BOT]);
  await settle();
  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  unmount();
  roster.teardown();
});

test("huddle sync records changes made before it failed", async () => {
  const roster = setup("huddle-parent-partial");
  const unmount = await cacheOldRoster(roster);
  nativeResults.sync_agents_to_active_huddle = {
    matched_active_huddle: true,
    added: [],
    changed_channel_ids: ["huddle-parent-partial"],
    error: "agent add rejected",
  };
  await assert.rejects(
    syncAgentsToActiveHuddle("huddle-parent-partial", [BOT]),
    {
      message: "agent add rejected",
    },
  );
  await settle();
  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  unmount();
  roster.teardown();
});

test("a Bestie DM's first roster read goes to the writer", async () => {
  const roster = setup("bestie-dm");
  nativeResults.resolve_bestie_conversation = {
    id: "bestie-dm",
    name: "bestie",
    channel_type: "dm",
    visibility: "private",
    participants: [],
    participant_pubkeys: [],
  };
  await resolveBestieConversation({
    expectedRelayUrl: "wss://relay.test",
    expectedSignerPubkey: OWNER,
  });
  const unmount = roster.mount();
  await settle();
  assert.deepEqual(routes, ["writer"]);
  assert.deepEqual(roster.shown(), [OWNER, BOT]);
  unmount();
  roster.teardown();
});

test("a change recorded before the listener registers still refreshes", async () => {
  routes.length = 0;
  const queryClient = new QueryClient();
  // Cached old roster, then a change while no listener exists.
  queryClient.setQueryData(channelMembersQueryKey("early"), [
    { pubkey: OWNER, role: "member" },
  ]);
  const observer = new QueryObserver(queryClient, {
    queryKey: channelMembersQueryKey("early"),
    queryFn: () => getChannelMembers("early"),
    staleTime: 5 * 60_000,
  });
  const unmount = observer.subscribe(() => {});
  writerMembers = [OWNER, BOT];
  noteChannelMembershipChange("early");
  const teardown = refreshRostersOnMembershipChange(queryClient);
  await settle();
  assert.deepEqual(
    queryClient
      .getQueryData(channelMembersQueryKey("early"))
      ?.map((member) => member.pubkey),
    [OWNER, BOT],
  );
  unmount();
  teardown();
});

test("ordinary reads return to the replica once the window passes", async () => {
  const roster = setup("ch-later");
  await addChannelMembers({
    channelId: "ch-later",
    pubkeys: [BOT],
    role: "bot",
  });
  now += 31_001;
  await getChannelMembers("ch-later");
  await getChannelMembers("ch-other");
  assert.deepEqual(routes.slice(-2), ["replica", "replica"]);
  roster.teardown();
});

test("a community switch clears recorded changes", async () => {
  const roster = setup("ch-community");
  await addChannelMembers({
    channelId: "ch-community",
    pubkeys: [BOT],
    role: "bot",
  });
  resetChannelMembershipWrites();
  await getChannelMembers("ch-community");
  assert.equal(routes.at(-1), "replica");
  roster.teardown();
});
