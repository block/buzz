/**
 * Mounted contracts for bounded channel-reference resolution. These exercise
 * the real React Query hooks and Tauri boundary: a channel reference may fetch
 * one detail event, but must never start the all-open directory scan.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  HTMLElement: dom.window.HTMLElement,
  HTMLIFrameElement: dom.window.HTMLIFrameElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  MutationObserver: dom.window.MutationObserver,
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
// The discussion facepile renders UserProfilePopover, which mounts HuddleProvider;
// its audio-device effects touch navigator.mediaDevices, absent in jsdom.
Object.defineProperty(dom.window.navigator, "mediaDevices", {
  configurable: true,
  value: {
    addEventListener: () => {},
    enumerateDevices: async () => [],
    removeEventListener: () => {},
  },
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => ipc.invoke(command, args),
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
// @tauri-apps/api reads unregisterListener off window during listener teardown.
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ =
  globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;

const ipc = {
  detailCalls: [],
  directoryCalls: 0,
  detail: async () => {
    throw new Error("unconfigured detail response");
  },
  search: async () => ({ found: 0, hits: [] }),
  users: async () => ({ missing: [], profiles: {} }),
  async invoke(command, args) {
    if (command === "get_channel_details") {
      this.detailCalls.push(args.channelId);
      return this.detail(args.channelId);
    }
    if (command === "get_open_channel_directory") {
      this.directoryCalls += 1;
      return [];
    }
    if (command === "search_messages") return this.search(args);
    if (command === "get_users_batch") return this.users(args);
    // HuddleProvider (mounted transitively via the discussion facepile's
    // profile popover) registers Tauri event listeners. Absorb them so the
    // panel can render; its audio probes are all best-effort and swallow the
    // unmocked-command throw below.
    if (command.startsWith("plugin:event|")) return 0;
    throw new Error(`unmocked Tauri command: ${command}`);
  },
  reset() {
    this.detailCalls = [];
    this.directoryCalls = 0;
    this.detail = async () => {
      throw new Error("unconfigured detail response");
    };
    this.search = async () => ({ found: 0, hits: [] });
    this.users = async () => ({ missing: [], profiles: {} });
  },
};

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;
let HuddleProvider;
let useChannelReference;
let useSearchResults;
let channelReferenceQueryKey;
let channelsQueryKey;
let openChannelDirectoryQueryKey;
let isChannelReferenceOpenable;
let useChannelReferences;
let useOpenAgentActivity;
let useReminderSources;
let DiscussionChannelsPanel;
let createMarkdownComponents;
let renderCachedMarkdown;
let MarkdownRuntimeContext;
let relayAgentsQueryKey;
let createMemoryHistory;
let createRootRoute;
let createRoute;
let createRouter;
let RouterProvider;

const COMMUNITY = {
  addedAt: "2026-08-19T00:00:00.000Z",
  id: "reference-test-community",
  name: "Reference test",
  relayUrl: "ws://reference.test",
};
const VIEWER = "a".repeat(64);

function rawChannel({ id, name, visibility = "open" }) {
  return {
    archived_at: null,
    channel_type: "stream",
    description: "",
    id,
    is_member: false,
    last_message_at: null,
    member_count: 0,
    member_pubkeys: [],
    name,
    participant_pubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttl_deadline: null,
    ttl_seconds: null,
    visibility,
  };
}

function rawDetail(channel) {
  return {
    ...channel,
    created_at: "2026-08-19T00:00:00.000Z",
    created_by: VIEWER,
    max_members: null,
    nip29_group_id: null,
    purpose_set_at: null,
    purpose_set_by: null,
    topic_required: false,
    topic_set_at: null,
    topic_set_by: null,
    updated_at: "2026-08-19T00:00:00.000Z",
  };
}

function channel({ id, name, isMember = true, visibility = "open" }) {
  return {
    archivedAt: null,
    channelType: "stream",
    description: "",
    id,
    isMember,
    lastMessageAt: null,
    memberCount: 0,
    memberPubkeys: [],
    name,
    participantPubkeys: [],
    participants: [],
    purpose: null,
    topic: null,
    ttlDeadline: null,
    ttlSeconds: null,
    visibility,
  };
}

function createClient({ memberChannels = [], warmChannels } = {}) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Number.POSITIVE_INFINITY, retry: false },
    },
  });
  client.setQueryData(["identity"], { pubkey: VIEWER });
  client.setQueryData(channelsQueryKey, memberChannels);
  if (warmChannels) {
    client.setQueryData(openChannelDirectoryQueryKey, warmChannels);
  }
  return client;
}

async function mountReference(client, channelId) {
  let value;
  function Probe({ id }) {
    value = useChannelReference(id);
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const render = async (id) => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(Probe, { id }),
          ),
        ),
      );
    });
  };

  await render(channelId);
  return {
    get value() {
      return value;
    },
    render,
    async settle() {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
    },
    async unmount() {
      await act(async () => root.unmount());
      client.clear();
      container.remove();
    },
  };
}

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ HuddleProvider } = await import("@/features/huddle"));
  ({
    channelReferenceQueryKey,
    openChannelDirectoryQueryKey,
    useChannelReference,
    useChannelReferences,
  } = await import("./openChannelDirectory.ts"));
  ({ channelsQueryKey } = await import("./hooks.ts"));
  ({ relayAgentsQueryKey } = await import("@/features/agents/hooks.ts"));
  ({ useOpenAgentActivity } = await import(
    "@/features/agents/useOpenAgentActivity.ts"
  ));
  ({ useReminderSources } = await import(
    "@/features/reminders/ui/RemindersPanel.tsx"
  ));
  ({ DiscussionChannelsPanel } = await import(
    "@/features/projects/ui/DiscussionChannels.tsx"
  ));
  ({ createMarkdownComponents } = await import("@/shared/ui/markdown.tsx"));
  ({ renderCachedMarkdown } = await import(
    "@/shared/ui/markdown/nodeCache.ts"
  ));
  ({ MarkdownRuntimeContext } = await import(
    "@/shared/ui/markdown/runtimeContext.ts"
  ));
  ({
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    RouterProvider,
  } = await import("@tanstack/react-router"));
  ({ useSearchResults } = await import(
    "@/features/search/useSearchResults.ts"
  ));
  ({ isChannelReferenceOpenable } = await import("./openChannelDirectory.ts"));
});

beforeEach(() => {
  ipc.reset();
  localStorage.clear();
  localStorage.setItem("buzz-communities", JSON.stringify([COMMUNITY]));
  localStorage.setItem("buzz-active-community-id", COMMUNITY.id);
});

afterEach(() => ipc.reset());
after(() => dom.window.close());

test("opening global search with an empty query does not scan the open directory", async () => {
  const client = createClient();
  let search;
  function Probe() {
    search = useSearchResults({ channels: [], enabled: true });
    return null;
  }

  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(Probe),
        ),
      ),
    );
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(search.query, "");
  assert.equal(ipc.directoryCalls, 0);

  await act(async () => root.unmount());
  client.clear();
  container.remove();
});

test("an unknown id fetches one detail without scanning the open directory", async () => {
  const client = createClient();
  ipc.detail = async (channelId) =>
    rawDetail(rawChannel({ id: channelId, name: "remote" }));
  const mounted = await mountReference(client, "unknown-channel");

  await mounted.settle();

  assert.deepEqual(ipc.detailCalls, ["unknown-channel"]);
  assert.equal(ipc.directoryCalls, 0);
  assert.equal(mounted.value?.name, "remote");
  await mounted.unmount();
});

test("member and warm-directory references avoid the bounded detail request", async () => {
  const memberClient = createClient({
    memberChannels: [channel({ id: "member", name: "member" })],
  });
  const member = await mountReference(memberClient, "member");
  await member.settle();
  assert.equal(member.value?.name, "member");
  await member.unmount();

  const warmClient = createClient({
    warmChannels: [channel({ id: "warm", isMember: false, name: "warm" })],
  });
  const warm = await mountReference(warmClient, "warm");
  await warm.settle();
  assert.equal(warm.value?.name, "warm");
  assert.deepEqual(ipc.detailCalls, []);
  assert.equal(ipc.directoryCalls, 0);
  await warm.unmount();
});

test("fetched private metadata remains non-openable", async () => {
  const client = createClient();
  ipc.detail = async (channelId) =>
    rawDetail(
      rawChannel({ id: channelId, name: "private", visibility: "private" }),
    );
  const mounted = await mountReference(client, "private-channel");

  await mounted.settle();

  assert.equal(mounted.value?.isMember, false);
  assert.equal(mounted.value?.visibility, "private");
  assert.equal(isChannelReferenceOpenable(mounted.value), false);
  assert.equal(ipc.directoryCalls, 0);
  await mounted.unmount();
});

test("a not-found detail result is cached as a five-minute miss", async () => {
  const client = createClient();
  ipc.detail = async () => {
    throw new Error("channel not found");
  };
  const first = await mountReference(client, "missing-channel");
  await first.settle();

  assert.equal(first.value, undefined);
  assert.deepEqual(ipc.detailCalls, ["missing-channel"]);
  assert.equal(
    client.getQueryData(channelReferenceQueryKey("missing-channel")),
    null,
  );
  await first.unmount();

  const second = await mountReference(client, "missing-channel");
  await second.settle();
  assert.deepEqual(ipc.detailCalls, ["missing-channel"]);
  assert.equal(ipc.directoryCalls, 0);
  await second.unmount();
});

async function mountWithRouter(client, Component) {
  const rootRoute = createRootRoute({
    component: () => React.createElement(Component),
  });
  const channelRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/channels/$channelId",
    component: () => null,
  });
  const router = createRouter({
    routeTree: rootRoute.addChildren([channelRoute]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            HuddleProvider,
            null,
            React.createElement(RouterProvider, { router }),
          ),
        ),
      ),
    );
  });
  return {
    container,
    async settle() {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
    },
    async unmount() {
      await act(async () => root.unmount());
      client.clear();
      container.remove();
    },
  };
}

async function mountMarkdownReference(client, content, variant) {
  const markdown = renderCachedMarkdown({
    components: createMarkdownComponents(true, false),
    content,
    variant,
  });
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            MarkdownRuntimeContext.Provider,
            {
              value: {
                channels: [],
                onOpenChannel: () => {},
                onOpenEntityLink: () => {},
                onOpenMessageLink: () => {},
                relayOrigin: null,
                resolveChannelReferences: true,
              },
            },
            markdown,
          ),
        ),
      ),
    );
  });
  return {
    container,
    async settle() {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
    },
    async unmount() {
      await act(async () => root.unmount());
      client.clear();
      container.remove();
    },
  };
}

test("markdown message links resolve private destinations without a directory scan", async () => {
  const channelId = "private-markdown-channel";
  const messageId = "e".repeat(64);
  const link = `buzz://message?channel=${channelId}&id=${messageId}`;
  const renderPaths = [
    ["CommonMark autolink", `<${link}>`],
    ["bare message-link node", link],
  ];

  for (const [path, content] of renderPaths) {
    const client = createClient();
    ipc.detail = async (id) =>
      rawDetail(rawChannel({ id, name: "private", visibility: "private" }));
    const mounted = await mountMarkdownReference(
      client,
      content,
      `private-message-link-${path}`,
    );
    await mounted.settle();

    assert.deepEqual(ipc.detailCalls, [channelId], path);
    assert.equal(ipc.directoryCalls, 0, path);
    assert.equal(
      mounted.container.querySelector("button[data-message-link]"),
      null,
      `${path} private destination must not render a clickable pill`,
    );
    assert.notEqual(
      mounted.container.querySelector(
        "span[data-message-link][data-buzz-link]",
      ),
      null,
      `${path} private destination must render an inert message-link pill`,
    );
    await mounted.unmount();
    ipc.reset();
  }
});

test("authored-label channel and message links respect the private-destination gate", async () => {
  // Authored-label deep links must route through the same bounded detail
  // lookup + openable gate as the pill paths, regardless of parser family:
  //   - buzz://channel/<uuid> and buzz://channel/<uuid>/<event-id> reach the
  //     gate via ChannelDeepLinkAnchor's authored branch, and
  //   - the canonical buzz://message?channel=&id= form (produced by
  //     buildMessageLink) reaches it via resolveMessageLinkRenderTarget's
  //     "label" branch.
  // A private channel must render inert on every route regardless of the
  // display text.
  const channelId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const messageId = "b".repeat(64);
  const channelLink = `buzz://channel/${channelId}`;
  const channelMessageLink = `buzz://channel/${channelId}/${messageId}`;
  const canonicalMessageLink = `buzz://message?channel=${channelId}&id=${messageId}`;
  const renderPaths = [
    ["channel variant", `[private channel](${channelLink})`],
    [
      "channel-path message variant",
      `[private message](${channelMessageLink})`,
    ],
    ["canonical message variant", `[private message](${canonicalMessageLink})`],
  ];

  for (const [path, content] of renderPaths) {
    const client = createClient();
    ipc.detail = async (id) =>
      rawDetail(rawChannel({ id, name: "private", visibility: "private" }));
    const mounted = await mountMarkdownReference(
      client,
      content,
      `private-authored-label-${path}`,
    );
    await mounted.settle();

    assert.deepEqual(ipc.detailCalls, [channelId], `${path}: bounded detail`);
    assert.equal(ipc.directoryCalls, 0, `${path}: no directory scan`);
    assert.equal(
      mounted.container.querySelector("button"),
      null,
      `${path} private destination must not render a clickable element`,
    );
    assert.notEqual(
      mounted.container.querySelector("span[data-buzz-link]"),
      null,
      `${path} private destination must render an inert node`,
    );
    await mounted.unmount();
    ipc.reset();
  }
});

test("multi-id references dedupe cold ids and share the single-id query cache", async () => {
  const client = createClient();
  ipc.detail = async (channelId) =>
    rawDetail(rawChannel({ id: channelId, name: `#${channelId}` }));
  let references;
  function Probe() {
    references = useChannelReferences(["cold", "cold", "other"]);
    return null;
  }
  const mounted = await mountWithRouter(client, Probe);
  await mounted.settle();

  assert.deepEqual(ipc.detailCalls.sort(), ["cold", "other"]);
  assert.equal(references.channelsById.get("cold")?.name, "#cold");
  assert.equal(ipc.directoryCalls, 0);
  await mounted.unmount();
});

test("agent activity opens a cold readable channel without a directory scan", async () => {
  const client = createClient();
  const agentPubkey = "b".repeat(64);
  client.setQueryData(relayAgentsQueryKey, [
    {
      pubkey: agentPubkey,
      ownerPubkey: VIEWER,
      name: "Agent",
      agentType: "agent",
      channels: [],
      channelIds: ["cold-agent-channel"],
      capabilities: [],
      status: "online",
      respondTo: null,
      respondToAllowlist: [],
    },
  ]);
  ipc.detail = async (channelId) =>
    rawDetail(rawChannel({ id: channelId, name: "cold-agent" }));
  let activity;
  function Probe() {
    activity = useOpenAgentActivity();
    return null;
  }
  const mounted = await mountWithRouter(client, Probe);
  await mounted.settle();

  assert.equal(activity.canOpenAgentActivity(agentPubkey), true);
  assert.equal(activity.openAgentActivity(agentPubkey), true);
  assert.deepEqual(ipc.detailCalls, ["cold-agent-channel"]);
  assert.equal(ipc.directoryCalls, 0);
  await mounted.unmount();
});

test("reminder sources label a cold readable channel without a directory scan", async () => {
  const client = createClient();
  const reminder = {
    id: "reminder",
    eventId: "event",
    createdAt: 1,
    content: {
      status: "pending",
      target: {
        eventId: "message",
        channelId: "cold-reminder-channel",
        preview: "Reminder source",
        authorPubkey: "c".repeat(64),
      },
    },
  };
  ipc.detail = async (channelId) =>
    rawDetail(rawChannel({ id: channelId, name: "cold-reminder" }));
  let sources;
  function Probe() {
    sources = useReminderSources([reminder]);
    return null;
  }
  const mounted = await mountWithRouter(client, Probe);
  await mounted.settle();

  assert.equal(sources.get("reminder")?.channelLabel, "cold-reminder");
  assert.deepEqual(ipc.detailCalls, ["cold-reminder-channel"]);
  assert.equal(ipc.directoryCalls, 0);
  await mounted.unmount();
});

test("discussion rows label a cold readable channel without a directory scan", async () => {
  const client = createClient();
  ipc.search = async () => ({
    found: 1,
    hits: [
      {
        event_id: "event",
        content: "discussion",
        kind: 9,
        pubkey: "d".repeat(64),
        channel_id: "abc12345-cold-discussion-channel",
        channel_name: null,
        created_at: 1,
        score: 1,
      },
    ],
  });
  ipc.detail = async (channelId) =>
    rawDetail(rawChannel({ id: channelId, name: "cold-discussion" }));
  const mounted = await mountWithRouter(client, () =>
    React.createElement(DiscussionChannelsPanel, {
      query: "discussion query",
      repositoryName: "repo",
    }),
  );
  await mounted.settle();
  await mounted.settle();

  assert.match(mounted.container.textContent, /#cold-discussion/);
  assert.doesNotMatch(mounted.container.textContent, /#abc12345/);
  assert.deepEqual(ipc.detailCalls, ["abc12345-cold-discussion-channel"]);
  assert.equal(ipc.directoryCalls, 0);
  await mounted.unmount();
});
