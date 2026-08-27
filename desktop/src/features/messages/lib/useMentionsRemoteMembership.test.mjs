import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { JSDOM } from "jsdom";

// ── Remote-agent membership badge regression ─────────────────────────────────
//
// Repro shape (Zenbook, 2026-08-26): an agent managed on another install joins
// a channel. The relay's live agent directory lists it with that channel in
// `channelIds` — the same relay-signed fact that admits the agent into
// autocomplete (relayAgentCanRespondInChannel) and authorizes mention
// delivery. But the client's member roster (get_channel_members) is served
// from a react-query cache with a 5-minute freshness window, so it can predate
// the join. The relay-agent candidate loop in useMentions hardcodes
// `isMember: false`, and mentionSuggestionMapping derives `notInChannel` from
// `isMember === false` — so an admitted, deliverable agent gets badged
// "not in channel" until the roster cache lapses.
//
// These tests drive the real hook with a stale roster (no trace of the remote
// agent) and a live directory (channelIds contains the channel) and assert
// the badge is not applied. The roster-present and managed-outside cases pin
// the fix's scope.

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
  localStorage: dom.window.localStorage,
});

const VIEWER = "a".repeat(64);
const CHANNEL_ID = "channel-general";
const REMOTE_AGENT = "b".repeat(64);
const REMOTE_OWNER = "d".repeat(64);
const MANAGED_AGENT = "e".repeat(64);

/** @type {Map<string, (args: any) => Promise<any>>} */
const ipc = new Map();

window.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    const handler = ipc.get(command);
    if (handler) return handler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback: () => Math.floor(Math.random() * 1e9),
};

// Production imports after the DOM/IPC shims are in place.
const { act, cleanup, renderHook, waitFor } = await import(
  "@testing-library/react"
);
const { default: React } = await import("react");
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { CommunitiesProvider } = await import(
  "@/features/communities/useCommunities.tsx"
);
const { useMentions } = await import("./useMentions.ts");

function memberRaw(pubkey, overrides = {}) {
  return {
    pubkey,
    role: "admin",
    is_agent: false,
    joined_at: "2026-08-26T00:00:00Z",
    display_name: "Leo",
    ...overrides,
  };
}

function remoteAgentRaw(overrides = {}) {
  return {
    pubkey: REMOTE_AGENT,
    owner_pubkey: REMOTE_OWNER,
    name: "Jarvis",
    agent_type: "acp",
    channels: [],
    channel_ids: [CHANNEL_ID],
    capabilities: [],
    status: "online",
    respond_to: "anyone",
    respond_to_allowlist: [],
    ...overrides,
  };
}

function managedAgentRaw(overrides = {}) {
  return {
    pubkey: MANAGED_AGENT,
    name: "LocalBot",
    persona_id: null,
    runtime: null,
    team_id: null,
    relay_url: "ws://reference.test",
    acp_command: "acp",
    agent_command: null,
    agent_args: [],
    mcp_command: null,
    turn_timeout_seconds: 60,
    idle_timeout_seconds: 60,
    max_turn_duration_seconds: 120,
    parallelism: 1,
    system_prompt: "prompt",
    avatar_url: null,
    model: null,
    status: "stopped",
    pid: null,
    created_at: "2026-08-26T00:00:00Z",
    updated_at: "2026-08-26T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    log_path: null,
    start_on_app_launch: false,
    backend: null,
    backend_agent_id: null,
    respond_to: "owner-only",
    respond_to_allowlist: [],
    ...overrides,
  };
}

function installDirectory({ relayAgents, members, managedAgents = [] }) {
  ipc.clear();
  ipc.set("get_identity", async () => ({
    pubkey: VIEWER,
    display_name: "Leo",
  }));
  ipc.set("get_channel_members", async () => ({
    members,
    next_cursor: null,
  }));
  ipc.set("list_relay_agents", async () => relayAgents);
  ipc.set("list_managed_agents", async () => managedAgents);
  ipc.set("list_personas", async () => []);
  ipc.set("list_teams", async () => []);
  ipc.set("list_archived_identities", async () => ({ archived: [] }));
  ipc.set("get_users_batch", async ({ pubkeys }) => ({
    profiles: {},
    missing: pubkeys ?? [],
  }));
  ipc.set("search_users", async () => ({ users: [], next_cursor: null }));
  ipc.set("revalidate_relay_agents", async () => []);
}

function mountMentions() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Number.POSITIVE_INFINITY },
    },
  });
  queryClients.push(client);
  return renderHook(
    () =>
      useMentions(CHANNEL_ID, undefined, undefined, { channelType: "stream" }),
    {
      wrapper: ({ children }) =>
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(CommunitiesProvider, null, children),
        ),
    },
  );
}

async function openPicker(view, query) {
  await waitFor(
    () => assert.equal(view.result.current.hasResolvedMembers, true),
    {
      timeout: 3000,
    },
  );
  await act(async () => {
    view.result.current.updateMentionQuery(`@${query}`, 1 + query.length);
  });
  // Flush the 120ms mention debounce inside act so the query state settles.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 250));
  });
  await waitFor(
    () =>
      assert.ok(
        view.result.current.suggestions.length > 0,
        "suggestions appear",
      ),
    { timeout: 3000 },
  );
}

// QueryClients created by mountMentions. React-query schedules a 5-minute GC
// timer per query when its last observer unmounts; without clear() those
// timers keep the event loop (and the test runner) alive for ~300s after the
// last assertion. Each test clears its own clients before the next mount.
const queryClients = [];

afterEach(async () => {
  cleanup();
  for (const client of queryClients.splice(0)) {
    await client.clear();
  }
  dom.window.localStorage.clear();
});

test("admitted remote agent is not badged not-in-channel when the roster cache predates its join", async () => {
  // Stale roster: the cached member list has no trace of the remote agent,
  // but the relay's live directory (`channel_ids`) says it is in the
  // channel — the same fact that admits it into autocomplete and authorizes
  // delivery.
  installDirectory({
    relayAgents: [remoteAgentRaw()],
    members: [memberRaw(VIEWER)],
  });
  const view = mountMentions();
  await openPicker(view, "jar");
  const suggestion = view.result.current.suggestions.find(
    (item) => item.displayName === "Jarvis",
  );
  assert.ok(suggestion, "remote agent is admitted into autocomplete");
  assert.equal(suggestion.isAgent, true);
  assert.equal(
    suggestion.notInChannel,
    false,
    "the stale roster cache must not badge a directory-confirmed member as outside the channel",
  );
});

test("remote agent present in a fresh roster keeps no not-in-channel badge", async () => {
  // Fresh roster: the merge path already marks the candidate as a member.
  // Guards against regressing the roster-present case while fixing the
  // stale-roster case above.
  installDirectory({
    relayAgents: [remoteAgentRaw()],
    members: [
      memberRaw(VIEWER),
      memberRaw(REMOTE_AGENT, {
        role: "bot",
        is_agent: true,
        display_name: "Jarvis",
      }),
    ],
  });
  const view = mountMentions();
  await openPicker(view, "jar");
  const suggestion = view.result.current.suggestions.find(
    (item) => item.displayName === "Jarvis",
  );
  assert.ok(suggestion, "remote agent is admitted into autocomplete");
  assert.equal(suggestion.notInChannel, false);
});

test("local managed agent outside the roster keeps the not-in-channel badge", async () => {
  // The fix must stay scoped: a locally managed agent that genuinely is not
  // a member of the channel still carries the badge.
  installDirectory({
    relayAgents: [],
    members: [memberRaw(VIEWER)],
    managedAgents: [managedAgentRaw()],
  });
  const view = mountMentions();
  await openPicker(view, "loc");
  const suggestion = view.result.current.suggestions.find(
    (item) => item.displayName === "LocalBot",
  );
  assert.ok(suggestion, "managed agent is admitted into autocomplete");
  assert.equal(suggestion.isAgent, true);
  assert.equal(suggestion.notInChannel, true);
});
