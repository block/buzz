// Admin Console kicks bypass the channel-event wrappers, so the member list
// must still refresh from the writer: from the relay's live `admin_kick`
// system message, and from the initiating resolve when it confirms the kick.
import assert from "node:assert/strict";
import { after, mock, test } from "node:test";
import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import React from "react";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
after(() => dom.window.close());

const OWNER = "aa".repeat(32);
const BOT = "bb".repeat(32);
const id = "00000000-0000-0000-0000-0000000000c1";
let writer;
let replica;
let routes;
window.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    if (command === "get_relay_ws_url") return "wss://alpha.example.com";
    if (command === "get_channel_window") return [];
    if (command === "get_channel_members") {
      const strong = args.readYourWrites === true;
      routes.push(strong ? "writer" : "replica");
      return {
        members: (strong ? writer : replica).map((pubkey) => ({
          pubkey,
          role: "member",
        })),
      };
    }
    if (command === "admin_resolve_report") {
      writer = [OWNER];
      return {
        status: "resolved",
        activeAction: { action: "kick", status: "succeeded" },
      };
    }
    return null;
  },
};

const { useChannelSubscription } = await import("@/features/messages/hooks");
const { relayClient } = await import("@/shared/api/relayClient");
const { getChannelMembers } = await import("@/shared/api/tauri");
const { resolveAdminReport } = await import("@/features/admin-console/api");
const { resetChannelMembershipWrites } = await import(
  "@/shared/api/channelMembershipWrites"
);
const { channelMembersQueryKey, refreshRostersOnMembershipChange } =
  await import("./rosterFreshness.ts");
const { KIND_SYSTEM_MESSAGE } = await import("@/shared/constants/kinds");

async function settle() {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

// Mounts a cached owner+bot roster, then runs `act` and returns the roster.
async function rosterAfter(act) {
  resetChannelMembershipWrites();
  writer = [OWNER, BOT];
  replica = [OWNER, BOT];
  routes = [];
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const stop = refreshRostersOnMembershipChange(client);
  const observer = new QueryObserver(client, {
    queryKey: channelMembersQueryKey(id),
    queryFn: () => getChannelMembers(id),
    staleTime: 300_000,
  });
  const unsubscribe = observer.subscribe(() => {});
  try {
    await settle();
    await act(client);
    await settle();
    return client
      .getQueryData(channelMembersQueryKey(id))
      ?.map((member) => member.pubkey);
  } finally {
    unsubscribe();
    stop();
    client.clear();
    mock.restoreAll();
  }
}

test("a live admin_kick system message refreshes the roster from the writer", async () => {
  const { renderHook, act, cleanup } = await import("@testing-library/react");
  let receive;
  mock.method(relayClient, "subscribeToReconnects", () => () => {});
  mock.method(relayClient, "subscribeToChannelLive", async (_, cb) => {
    receive = cb;
    return async () => {};
  });
  const channel = {
    id,
    name: "general",
    channelType: "stream",
    visibility: "open",
    isMember: true,
  };
  const shown = await rosterAfter(async (client) => {
    const view = renderHook(() => useChannelSubscription(channel), {
      wrapper: ({ children }) =>
        React.createElement(QueryClientProvider, { client }, children),
    });
    await act(async () => await settle());
    writer = [OWNER];
    await act(async () => {
      receive({
        id: "kick-event",
        pubkey: OWNER,
        created_at: 20,
        kind: KIND_SYSTEM_MESSAGE,
        tags: [["h", id]],
        content: JSON.stringify({ type: "admin_kick", target: BOT }),
        sig: "",
      });
      await settle();
    });
    view.unmount();
    cleanup();
  });
  assert.deepEqual(shown, [OWNER]);
  assert.equal(routes.at(-1), "writer");
});

for (const { host, recorded } of [
  { host: "Alpha.example.com:443", recorded: true },
  { host: "beta.example.com", recorded: false },
]) {
  test(`a confirmed report kick in ${host} ${recorded ? "refreshes" : "leaves"} the active community's roster`, async () => {
    const shown = await rosterAfter(() =>
      resolveAdminReport(
        "https://admin.invalid",
        { id: "report", channelId: id, communityHost: host },
        { action: "kick", requestId: "request" },
      ),
    );
    assert.deepEqual(shown, recorded ? [OWNER] : [OWNER, BOT]);
    assert.deepEqual(routes, recorded ? ["replica", "writer"] : ["replica"]);
  });
}
