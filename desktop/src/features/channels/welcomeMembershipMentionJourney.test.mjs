// Real Welcome provisioning and mention discovery with fixture Tauri IPC.
// Native admission is modeled as independent per-key acceptance, not proved here.
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  HTMLElement: dom.window.HTMLElement,
  HTMLIFrameElement: dom.window.HTMLIFrameElement,
  MutationObserver: dom.window.MutationObserver,
  IS_REACT_ACT_ENVIRONMENT: true,
  self: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
const VIEWER = "a".repeat(64),
  AGENT = "b".repeat(64),
  OTHER = "c".repeat(64);
const CHANNEL = "11111111-1111-4111-8111-111111111111";
localStorage.setItem(
  "buzz-communities",
  JSON.stringify([
    {
      id: "test",
      name: "Test",
      relayUrl: "ws://test.invalid",
      addedAt: "2026-01-01T00:00:00Z",
    },
  ]),
);
localStorage.setItem("buzz-active-community-id", "test");
let state;
const starterKeys = [AGENT, OTHER, "d".repeat(64)];
const starters = ["Fizz", "Honey", "Pollen"].map((name, index) => ({
  pubkey: starterKeys[index],
  name,
  persona_id: ["builtin:fizz", "builtin:honey", "builtin:bumble"][index],
  team_id: "builtin-team:welcome",
  relay_url: "ws://test.invalid",
  agent_command: "buzz-agent",
  agent_args: [],
  mcp_command: "",
  model: null,
  provider: null,
  status: "stopped",
  backend: { type: "local" },
  respond_to: "owner-only",
  respond_to_allowlist: [],
}));
const channel = () => ({
  id: CHANNEL,
  name: "fresh",
  channel_type: "stream",
  visibility: "open",
  description: "",
  is_member: true,
  archived_at: null,
  member_pubkeys: state.visible ? [VIEWER, AGENT] : [VIEWER],
  member_count: state.visible ? 2 : 1,
  participant_pubkeys: [],
  participants: [],
  last_message_at: null,
  ttl_seconds: null,
  ttl_deadline: null,
});
const invoke = async (command, args) => {
  if (command.startsWith("plugin:event|")) return 0;
  if (command === "search_users") {
    return { users: [], next_cursor: null };
  }
  if (command === "get_identity") return { pubkey: state.signer };
  if (command === "create_channel") return channel();
  if (command === "get_channels")
    return {
      channels: [channel()],
      hash: String(state.visible),
      last_messages: [],
    };
  if (command === "get_channel_members") {
    state.rosterCalls += 1;
    if (state.rosterGate) {
      const gate = state.rosterGate;
      state.rosterGate = null;
      state.rosterWaiting = true;
      await gate;
    }
    return {
      members: [
        {
          pubkey: VIEWER,
          role: "owner",
          display_name: "Viewer",
          is_agent: false,
        },
        ...starters
          .filter((agent) => state.members.has(agent.pubkey))
          .map((agent) => ({
            pubkey: agent.pubkey,
            role: "bot",
            display_name: agent.name,
            is_agent: true,
          })),
      ],
    };
  }
  if (command === "add_channel_members") {
    assert.equal(args.channelId, CHANNEL);
    assert.equal(args.role, "bot");
    state.addCalls.push(args.pubkeys);
    state.addScopes.push([args.expectedRelayUrl, args.expectedSignerPubkey]);
    // Native checks and captures authority BEFORE the per-key admission awaits.
    if (args.expectedRelayUrl && args.expectedRelayUrl !== state.relay)
      throw new Error("relay changed");
    if (args.expectedSignerPubkey && args.expectedSignerPubkey !== state.signer)
      throw new Error("signer changed");
    if (state.addGate) await state.addGate;
    if (state.reject) throw new Error("transport rejected");
    const added = args.pubkeys.filter((key) => state.allowed.has(key));
    for (const key of added) state.members.add(key);
    if (state.rejectAfterCommit)
      throw new Error("transport rejected after commit");
    return {
      added,
      errors: args.pubkeys
        .filter((key) => !state.allowed.has(key))
        .map((pubkey) => ({ pubkey, error: "admission denied" })),
    };
  }
  if (command === "list_managed_agents") return starters;
  if (command === "list_personas")
    return starters.map((agent) => ({
      id: agent.persona_id,
      display_name: agent.name,
      is_builtin: true,
      is_active: true,
      system_prompt: "Welcome",
      avatar_url: null,
    }));
  if (command === "discover_acp_providers")
    return [
      {
        id: "buzz-agent",
        label: "Buzz Agent",
        availability: "available",
        command: "buzz-agent",
        default_args: [],
        mcp_command: null,
      },
    ];
  if (command === "get_global_agent_config")
    return { preferred_runtime: "buzz-agent" };
  if (command === "agent_access_owner_only") return true;
  if (command === "sync_agents_to_active_huddle") return null;
  if (command === "list_relay_agents") {
    state.directoryCalls += 1;
    return [];
  }
  if (command === "revalidate_relay_agents") return [];
  if (["list_managed_agents", "list_personas", "list_teams"].includes(command))
    return [];
  if (command === "get_users_batch") return { profiles: {}, missing: [] };
  if (command === "list_archived_identities") return { archived: [] };
  throw new Error(`Unexpected IPC: ${command}`);
};
globalThis.__TAURI_INTERNALS__ = { invoke, transformCallback: () => 1 };
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ =
  globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;

let React,
  act,
  createRoot,
  QueryClient,
  QueryClientProvider,
  CommunitiesProvider;
let useMentions, resetMembershipDirectorySync;
let root, client, mention, ensureWelcomeTeam, waitFor;
before(async () => {
  ({ ensureWelcomeTeam } = await import(
    "@/features/onboarding/welcomeGuide.ts"
  ));
  ({ waitFor } = await import("@testing-library/react"));
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ useMentions } = await import("@/features/messages/lib/useMentions.ts"));
  ({ resetMembershipDirectorySync } = await import(
    "./membershipDirectorySync.ts"
  ));
});
function Composer() {
  mention = useMentions(state.channelId, undefined, undefined, {
    channelType: "stream",
  });
  return null;
}
async function render(withComposer = true) {
  await act(async () =>
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          withComposer ? React.createElement(Composer) : null,
        ),
      ),
    ),
  );
}
async function settled(assertion) {
  await waitFor(assertion);
}
async function setup(overrides = {}) {
  state = {
    signer: VIEWER,
    relay: "ws://test.invalid",
    addScopes: [],
    members: new Set(),
    allowed: new Set(),
    addCalls: [],
    rosterCalls: 0,
    channelId: CHANNEL,
    role: "bot",
    owner: VIEWER,
    policy: "anyone",
    visible: false,
    directoryCalls: 0,
    ...overrides,
  };
  client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
  for (const [key, data] of [
    [["identity"], { pubkey: VIEWER }],
    [["channels"], []],
    [["managed-agents"], []],
    [["relay-agents"], []],
    [["personas"], []],
    [["teams"], []],
    [["archivedIdentities"], { archived: [] }],
  ])
    client.setQueryData(key, data);
  const container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await render();
  await act(async () => mention.updateMentionQuery("@", 1));
  await settled(() => assert.equal(mention.isMentionLoading, false));
}
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  resetMembershipDirectorySync();
  client?.clear();
  document.body.replaceChildren();
});
after(() => dom.window.close());

for (const outcome of ["partial", "none", "rejected", "full"]) {
  test(`Welcome ${outcome} admission refreshes authoritative discovery and permits retry`, async () => {
    await setup({
      allowed: new Set(
        outcome === "full" ? starterKeys : outcome === "partial" ? [AGENT] : [],
      ),
      reject: outcome === "rejected",
    });
    const installed = mention.suggestions.map((row) => [row.pubkey, row.name]);
    const beforeCalls = state.rosterCalls;
    await act(async () => {
      const attempt = ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client);
      const duplicate = ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client);
      const results = await Promise.allSettled([attempt, duplicate]);
      if (outcome === "full") {
        assert.equal(results[0].status, "fulfilled");
        assert.deepEqual(results[1], results[0]);
      } else {
        assert.equal(results[0].status, "rejected");
        assert.match(
          results[0].reason.message,
          outcome === "rejected" ? /transport rejected/ : /admission denied/,
        );
        assert.equal(results[1].status, "rejected");
        assert.equal(results[1].reason, results[0].reason);
      }
    });
    assert.deepEqual(state.addCalls, [starterKeys]);
    await settled(() => {
      assert.ok(
        state.rosterCalls >= beforeCalls + 2,
        "provision read plus cache refetch",
      );
      assert.deepEqual(
        [...mention.memberPubkeys].sort(),
        [VIEWER, ...state.allowed].sort(),
      );
    });
    assert.deepEqual(
      mention.suggestions.map((row) => [row.pubkey, row.name]),
      installed,
      "background refresh must preserve installed choice identities and order",
    );
    await act(async () => mention.updateMentionQuery("", 0));
    await act(async () => mention.updateMentionQuery("@", 1));
    await settled(() => {
      assert.equal(mention.isMentionLoading, false);
      for (const key of starterKeys) {
        const memberRows = mention.suggestions.filter(
          (row) => row.pubkey === key && !row.notInChannel,
        );
        assert.equal(memberRows.length, state.allowed.has(key) ? 1 : 0);
      }
    });
    // Provisioning failures release the in-flight slot. Retry only missing keys.
    const previouslyAccepted = [...state.members];
    state.allowed = new Set(starterKeys);
    state.reject = false;
    await act(async () =>
      ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client),
    );
    assert.deepEqual(
      state.addCalls.at(-1),
      outcome === "full"
        ? starterKeys
        : starterKeys.filter((key) => !previouslyAccepted.includes(key)),
    );
    await settled(() =>
      assert.deepEqual(
        [...mention.memberPubkeys].sort(),
        [VIEWER, ...starterKeys].sort(),
      ),
    );
    await render(false);
    await render();
    await act(async () => mention.updateMentionQuery("@", 1));
    await settled(() => {
      assert.equal(mention.isMentionLoading, false);
      for (const key of starterKeys)
        assert.equal(
          mention.suggestions.filter(
            (row) => row.pubkey === key && !row.notInChannel,
          ).length,
          1,
        );
    });
  });
}

// Reproduces App's replacement scoped QueryClient while admission is pending.
for (const outcome of [
  "partial",
  "none",
  "rejected",
  "rejected-after-commit",
  "full",
]) {
  test(`replacement scoped client discovers settled Welcome ${outcome} admission on reopen`, async () => {
    await setup({
      allowed: new Set(
        outcome === "full"
          ? starterKeys
          : ["partial", "rejected-after-commit"].includes(outcome)
            ? [AGENT]
            : [],
      ),
      reject: outcome === "rejected",
      rejectAfterCommit: outcome === "rejected-after-commit",
    });
    const oldClient = client;
    const { promise, resolve } = Promise.withResolvers();
    state.addGate = promise;
    const first = ensureWelcomeTeam(CHANNEL, "ws://test.invalid", oldClient);
    const firstResult = first.then(
      (value) => ({ value }),
      (error) => ({ error }),
    );
    try {
      await settled(() => assert.equal(state.addCalls.length, 1));
      await render(false);
      client = new QueryClient({
        defaultOptions: { queries: { retry: false, gcTime: Infinity } },
      });
      assert.notEqual(client, oldClient);
      await render();
      await act(async () => mention.updateMentionQuery("@", 1));
      await settled(() => {
        assert.equal(mention.isMentionLoading, false);
        assert.deepEqual([...mention.memberPubkeys], [VIEWER]);
      });
      const installed = mention.suggestions.map((row) => [
        row.pubkey,
        row.name,
      ]);
      const beforeSettlementCalls = state.rosterCalls;
      const unrelatedKey = ["channels", "unrelated", "members"];
      oldClient.setQueryData(unrelatedKey, []);
      client.setQueryData(unrelatedKey, []);
      const second = ensureWelcomeTeam(CHANNEL, "ws://test.invalid/", client);
      const secondResult = second.then(
        (value) => ({ value }),
        (error) => ({ error }),
      );
      await act(async () => {
        resolve();
        const [a, b] = await Promise.all([firstResult, secondResult]);
        if (outcome === "full") assert.equal(a.value, b.value);
        else {
          assert.match(
            a.error.message,
            outcome.startsWith("rejected")
              ? /transport rejected/
              : /admission denied/,
          );
          assert.equal(
            a.error,
            b.error,
            "both participants retain the provisioning rejection",
          );
        }
      });
      assert.deepEqual(
        state.addCalls,
        [starterKeys],
        "late participant shares one admission batch",
      );
      assert.deepEqual(
        [...state.members].sort(),
        [...state.allowed].sort(),
        "fixture authority accepted only allowed starters",
      );
      assert.deepEqual(
        mention.suggestions.map((row) => [row.pubkey, row.name]),
        installed,
        "settlement does not replace the installed picker",
      );
      await act(async () => mention.updateMentionQuery("", 0));
      await act(async () => mention.updateMentionQuery("@", 1));
      await settled(() => {
        assert.equal(mention.isMentionLoading, false);
        for (const key of starterKeys)
          assert.equal(
            mention.suggestions.filter(
              (row) => row.pubkey === key && !row.notInChannel,
            ).length,
            state.allowed.has(key) ? 1 : 0,
            "replacement client discovers accepted starters on explicit reopen",
          );
      });
      await settled(() =>
        assert.deepEqual(
          [...mention.memberPubkeys].sort(),
          [VIEWER, ...state.allowed].sort(),
        ),
      );
      assert.ok(
        state.rosterCalls > beforeSettlementCalls,
        "replacement active roster refetched even with no accepted keys",
      );
      for (const participant of [oldClient, client])
        assert.equal(
          participant.getQueryState(unrelatedKey).isInvalidated,
          false,
          "refresh targets only captured channel",
        );
      assert.equal(
        oldClient.getQueryState(["channels", CHANNEL, "members"]).isInvalidated,
        true,
        "retired participant's inactive roster is also stale",
      );
    } finally {
      resolve();
      await firstResult;
      oldClient.clear();
    }
  });
}

for (const scope of ["signer", "relay"]) {
  test(`Welcome rejects ${scope} switch before membership IPC and refreshes captured channel`, async () => {
    await setup({ allowed: new Set(starterKeys) });
    const gate = Promise.withResolvers();
    state.rosterGate = gate.promise;
    const attempt = ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client);
    const result = attempt.then(
      () => ({ ok: true }),
      (error) => ({ error }),
    );
    try {
      await settled(() => assert.equal(state.rosterWaiting, true));
      state[scope] = scope === "signer" ? OTHER : "ws://other.invalid";
      state.channelId = "other-channel";
      client.setQueryData(["channels", "other-channel", "members"], []);
      await act(async () => {
        gate.resolve();
        const outcome = await result;
        assert.match(
          outcome.error?.message ?? "fulfilled",
          new RegExp(`${scope} changed`),
        );
      });
      assert.equal(state.members.size, 0);
      assert.deepEqual(state.addScopes, [["ws://test.invalid", VIEWER]]);
      assert.equal(
        client.getQueryState(["channels", "other-channel", "members"])
          .isInvalidated,
        false,
      );
      state.signer = VIEWER;
      state.relay = "ws://test.invalid";
      state.channelId = CHANNEL;
      await act(async () =>
        ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client),
      );
      await settled(() =>
        assert.deepEqual(
          [...mention.memberPubkeys].sort(),
          [VIEWER, ...starterKeys].sort(),
        ),
      );
    } finally {
      gate.resolve();
      await result;
    }
  });
}

test("new signer cannot join an already admitted Welcome batch; same-signer participants still deduplicate", async () => {
  await setup({ allowed: new Set(starterKeys) });
  const gate = Promise.withResolvers();
  state.addGate = gate.promise;
  const first = ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client);
  const results = [first];
  try {
    await settled(() => assert.equal(state.addCalls.length, 1));
    state.signer = OTHER;
    results.push(ensureWelcomeTeam(CHANNEL, "ws://test.invalid", client));
    results.push(ensureWelcomeTeam(CHANNEL, "ws://test.invalid/", client));
    await settled(() => assert.equal(state.addCalls.length, 2));
    assert.deepEqual(state.addScopes, [
      ["ws://test.invalid", VIEWER],
      ["ws://test.invalid", OTHER],
    ]);
    await act(async () => {
      gate.resolve();
      await Promise.all(results);
    });
    await settled(() =>
      assert.deepEqual(
        [...mention.memberPubkeys].sort(),
        [VIEWER, ...starterKeys].sort(),
      ),
    );
  } finally {
    gate.resolve();
    await Promise.allSettled(results);
  }
});
