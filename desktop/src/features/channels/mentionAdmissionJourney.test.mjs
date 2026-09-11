import {
  getMentionSelectionHistory,
  resetMentionSelectionHistory,
} from "../messages/lib/mentionSelectionHistory.ts";
// Admission against existing root query evidence, without membership freshness production.
// Real mention and picker hooks; Tauri policy/classification are fixture evidence.
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
const rawAgent = () => ({
  pubkey: AGENT,
  owner_pubkey: state.owner,
  name: "Remote Scout",
  agent_type: "agent",
  channels: [],
  channel_ids: state.directoryVisible ? [CHANNEL] : [],
  capabilities: [],
  status: "offline",
  respond_to: state.policy,
  respond_to_allowlist: [],
});
const invoke = async (command, args) => {
  if (command.startsWith("plugin:event|")) return 0;
  if (command === "search_users") {
    if (state.pendingSearch?.[args.query])
      return state.pendingSearch[args.query];
    return { users: state.searchUsers ?? [], next_cursor: null };
  }
  if (command === "get_identity") return { pubkey: VIEWER };
  if (command === "create_channel") return channel();
  if (command === "get_channels")
    return {
      channels: [channel()],
      hash: String(state.visible),
      last_messages: [],
    };
  if (command === "get_channel_members" && state.heldRoster)
    return state.heldRoster;
  if (command === "get_channel_members")
    return {
      members: [
        {
          pubkey: VIEWER,
          role: "owner",
          display_name: "Viewer",
          is_agent: false,
        },
        ...(state.visible
          ? [
              {
                pubkey: AGENT,
                role: state.role,
                display_name: "Remote Scout",
                is_agent: true,
              },
            ]
          : []),
      ],
    };
  if (command === "add_channel_members") {
    assert.equal(args.channelId, CHANNEL);
    assert.equal(args.role, state.role);
    state.accepted = true;
    return state.addResult;
  }
  if (command === "sync_agents_to_active_huddle") return null;
  if (command === "list_relay_agents") {
    state.directoryCalls += 1;
    if (state.heldDirectory) return state.heldDirectory;
    if (state.failDirectory) throw new Error("Directory unavailable");
    return state.missingDirectory ? [] : [rawAgent()];
  }
  if (command === "revalidate_relay_agents")
    return state.missingDirectory ? [] : [rawAgent()];
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
let useMentions;
let root, client, mention, picker;
let useAgentAddressLockPicker, effects;
before(async () => {
  ({ useAgentAddressLockPicker } = await import(
    "@/features/messages/ui/useAgentAddressLockPicker.ts"
  ));
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ useMentions } = await import("@/features/messages/lib/useMentions.ts"));
});
function Composer() {
  mention = useMentions(state.channelId, undefined, undefined, {
    channelType: state.channelType ?? "stream",
  });
  picker = useAgentAddressLockPicker({
    mentions: mention,
    audience: {
      pubkeys: state.locked,
      addPubkey: (key) => effects.push(["pin", key]),
      removePubkey: (key) => effects.push(["remove", key]),
    },
    audienceScope: state.channelId,
    richText: { getPlainTextAndCursor: () => ({ text: "@", cursor: 1 }) },
    applyAutocompleteEdit: (edit) => effects.push(["edit", edit]),
    onAddressAgentMention: (row) => effects.push(["promote", row.pubkey]),
    onPulseAddressLock: () => {},
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
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 300));
  });
  // React Query notification batching may be enqueued by effects committed above.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
  });
}
const rows = () => mention.suggestions.filter((row) => row.pubkey === AGENT);
async function setup(overrides = {}) {
  effects = [];
  state = {
    locked: [],
    channelId: CHANNEL,
    role: "bot",
    owner: VIEWER,
    policy: "anyone",
    accepted: false,
    visible: false,
    directoryVisible: false,
    directoryCalls: 0,
    addResult: { added: [AGENT], errors: [] },
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
    [["channels"], [channel()]],
    [["managed-agents"], []],
    [["personas"], []],
    [["teams"], []],
    [["archivedIdentities"], { archived: [] }],
  ])
    if (
      !(
        (state.heldDirectory || state.coldDirectory) &&
        key[0] === "relay-agents"
      )
    )
      client.setQueryData(key, data);
  const container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await render();
  await settle();
  await act(async () => mention.updateMentionQuery("@", 1));
  await settle();
}
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  resetMentionSelectionHistory();
  client?.clear();
  document.body.replaceChildren();
});
after(() => dom.window.close());

for (const change of [
  "policy-denied",
  "late-error",
  "directory-removed",
  "member-removed",
]) {
  test(`a retained callback cannot bind after ${change}`, async () => {
    await setup({ owner: OTHER, visible: true, directoryVisible: true });
    const staleRow = rows()[0];
    const staleInsert = captureInsert(mention.selectMention);
    assert.equal(staleRow.isAgent, true);
    assert.equal(mention.canSelectMention(staleRow), true);
    if (change === "policy-denied") state.policy = "owner-only";
    if (change === "late-error") state.failDirectory = true;
    if (change.endsWith("removed")) state.missingDirectory = true;
    if (change === "member-removed") {
      state.visible = false;
      await act(async () =>
        client.invalidateQueries({
          queryKey: ["channels", CHANNEL, "members"],
        }),
      );
    }
    await act(async () =>
      client.invalidateQueries({ queryKey: ["relay-agents"] }),
    );
    await settle();
    let edit;
    await act(async () => {
      edit = staleInsert(staleRow, 1);
    });
    assert.equal(
      edit.insertText,
      "",
      "old actionable row must not establish intent",
    );
    assert.deepEqual(mention.knownNames, []);
    assert.equal(
      mention.isAgentPubkey(AGENT),
      true,
      "directory removal never turns a known agent into a human",
    );
  });
}

test("only an exact current target can be selected", async () => {
  await setup({ visible: true, directoryVisible: true });
  assert.equal(mention.canSelectMention(rows()[0]), true);
  for (const target of [
    { displayName: "Remote Scout" },
    { displayName: "Remote Scout", pubkey: OTHER },
  ]) {
    assert.equal(mention.canSelectMention(target), false);
    let edit;
    await act(async () => {
      edit = captureInsert(mention.selectMention)(target, 1);
    });
    assert.equal(edit.insertText, "");
  }
  assert.deepEqual(mention.knownNames, []);
});

// These exercise the real sibling picker + mention hook, not an admission stub.
test("retained explicit pin rejects latest policy denial without draft effects", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  const row = rows()[0],
    oldPin = picker.toggleAlwaysAddressAgent;
  state.policy = "owner-only";
  await act(async () =>
    client.invalidateQueries({ queryKey: ["relay-agents"] }),
  );
  await settle();
  assert.equal(rows().length, 1, "denial does not move the displayed row");
  await act(async () => oldPin(row));
  assert.deepEqual(effects, []);
  assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
  assert.deepEqual(mention.knownNames, []);
});

for (const returnToOrigin of [false, true]) {
  test(`retained pin and insertion reject another scope visit (return=${returnToOrigin})`, async () => {
    await setup({ visible: true, directoryVisible: true });
    const row = rows()[0],
      oldPin = picker.toggleAlwaysAddressAgent;
    const oldInsert = captureInsert(mention.selectMention);
    const oldSelect = picker.selectMentionSuggestion;
    state.channelId = "22222222-2222-4222-8222-222222222222";
    await render();
    if (returnToOrigin) {
      state.channelId = CHANNEL;
      await render();
    }
    let edit;
    await act(async () => {
      oldPin(row);
      oldSelect(row);
      edit = oldInsert(row, 1);
    });
    assert.deepEqual(effects, []);
    assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
    assert.equal(edit.insertText, "");
    assert.deepEqual(mention.knownNames, []);
  });
}

test("latest locked state permits removal after denial, including a retained toggle", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  const row = rows()[0],
    oldPin = picker.toggleAlwaysAddressAgent;
  state.locked = [AGENT];
  state.policy = "owner-only";
  await act(async () =>
    client.invalidateQueries({ queryKey: ["relay-agents"] }),
  );
  await settle();
  assert.equal(mention.canSelectMention(row), false);
  await act(async () => oldPin(row));
  assert.ok(
    effects.some(([effect, key]) => effect === "remove" && key === AGENT),
  );
  assert.ok(
    effects.every(
      ([effect, edit]) =>
        effect === "remove" || (effect === "edit" && edit.insertText === ""),
    ),
  );
  assert.deepEqual(mention.knownNames, []);
});

test("retained team cannot bind a removed exact member", async () => {
  await setup({ visible: true, directoryVisible: true });
  const persona = {
    id: "review-scout",
    displayName: "Remote Scout",
    isActive: true,
  };
  const team = {
    id: "team-review",
    name: "Review Team",
    isBuiltin: false,
    personaIds: [persona.id],
  };
  await act(async () => {
    client.setQueryData(["personas"], [persona]);
    client.setQueryData(
      ["managed-agents"],
      [
        {
          pubkey: AGENT,
          name: "Remote Scout",
          personaId: persona.id,
          status: "running",
        },
      ],
    );
    client.setQueryData(["teams"], [team]);
  });
  await settle();
  await act(async () => mention.openMentionPicker(1));
  const row = mention.suggestions.find((s) => s.kind === "team");
  assert.ok(row, JSON.stringify(mention.suggestions));
  assert.equal(row.teamMembers[0].pubkey, AGENT);
  const old = captureInsert(mention.selectMention);
  state.missingDirectory = true;
  await act(async () => client.setQueryData(["managed-agents"], []));
  await act(async () =>
    client.invalidateQueries({ queryKey: ["relay-agents"] }),
  );
  await settle();
  assert.equal(
    mention.suggestions.some((s) => s.pubkey === AGENT),
    true,
  );
  assert.ok(mention.suggestions.find((s) => s.kind === "team"));
  let edit;
  await act(async () => {
    edit = old(row, 1);
  });
  assert.deepEqual(mention.knownNames, []);
  assert.deepEqual(mention.getDraftMentionRefs(edit.insertText), []);
  assert.equal(
    edit.insertText,
    "",
    "removed exact team member must not establish intent",
  );
});

test("duplicate team members cannot mask a recipient set change", async () => {
  await setup({ visible: true, directoryVisible: true });
  const personas = ["one", "two"].map((id) => ({
    id,
    displayName: id,
    isActive: true,
  }));
  const team = {
    id: "duplicates",
    name: "Duplicates",
    isBuiltin: false,
    personaIds: ["one", "one"],
  };
  await act(async () => {
    client.setQueryData(["personas"], personas);
    client.setQueryData(
      ["managed-agents"],
      personas.map((p, i) => ({
        pubkey: i ? OTHER : AGENT,
        name: p.displayName,
        personaId: p.id,
        status: "running",
      })),
    );
    client.setQueryData(["teams"], [team]);
  });
  await settle();
  await act(async () => mention.openMentionPicker(1));
  const row = mention.suggestions.find((s) => s.kind === "team");
  assert.ok(row);
  assert.equal(
    new Set(row.teamMembers.map((m) => m.pubkey ?? m.personaId)).size,
    1,
  );
  const insert = captureInsert(mention.selectMention);
  await act(async () =>
    client.setQueryData(["teams"], [{ ...team, personaIds: ["one", "two"] }]),
  );
  await settle();
  assert.equal(
    new Set(
      mention.suggestions
        .find((s) => s.kind === "team")
        .teamMembers.map((m) => m.pubkey ?? m.personaId),
    ).size,
    1,
    "the displayed team is frozen but current recipient admission is not",
  );
  let edit;
  await act(async () => {
    edit = insert(row, 1);
  });
  assert.equal(edit.insertText, "");
  assert.deepEqual(mention.knownNames, []);
  assert.deepEqual(mention.getDraftMentionRefs(edit.insertText), []);
});

const keyboard = (key) => ({ key, nativeEvent: { key }, preventDefault() {} });
const person = (pubkey, name = "Scout") => ({
  pubkey,
  display_name: name,
  is_agent: false,
});

for (const channelType of ["stream", "dm"]) {
  test(`${channelType} without a destination does not wait for a disabled roster`, async () => {
    await setup({
      channelId: null,
      channelType,
      searchUsers: [person(OTHER, "Alice")],
    });
    assert.equal(
      client.getQueryState(["channels", "none", "members"]).status,
      "pending",
    );
    assert.equal(
      client.getQueryState(["channels", "none", "members"]).fetchStatus,
      "idle",
    );
    await act(async () => mention.updateMentionQuery("@Alice", 6));
    await settle();
    assert.equal(mention.isMentionLoading, false);
    const choice = mention.handleMentionKeyDown(keyboard("Tab")).suggestion;
    assert.equal(choice.pubkey, OTHER);
    let edit;
    await act(async () => {
      edit = captureInsert(mention.selectMention)(choice, 6);
    });
    assert.equal(mention.getDraftMentionRefs(edit.insertText)[0].pubkey, OTHER);
  });

  test(`${channelType} with a real pending roster still waits before admitting choices`, async () => {
    let release;
    await setup({
      channelType,
      heldRoster: new Promise((resolve) => {
        release = resolve;
      }),
      searchUsers: [person(OTHER, "Alice")],
    });
    await act(async () => mention.updateMentionQuery("@Alice", 6));
    await settle();
    assert.equal(mention.isMentionLoading, true);
    assert.deepEqual(mention.suggestions, []);
    assert.equal(
      mention.handleMentionKeyDown(keyboard("Tab")).suggestion,
      undefined,
    );
    await act(async () => release({ members: [] }));
    await settle();
    assert.equal(mention.isMentionLoading, false);
    assert.equal(
      mention.handleMentionKeyDown(keyboard("Tab")).suggestion.pubkey,
      OTHER,
    );
  });
}

for (const selectionKey of ["Enter", "Tab"]) {
  test(`background membership/search updates leave visible same-name rows and ${selectionKey} identity fixed`, async () => {
    await setup({
      owner: OTHER,
      searchUsers: [person(OTHER), person("e".repeat(64))],
    });
    await act(async () => mention.updateMentionQuery("@Scout", 6));
    await settle();
    const displayed = mention.suggestions;
    assert.equal(displayed.length, 2);
    assert.deepEqual(
      new Set(displayed.map((row) => row.pubkey)),
      new Set([OTHER, "e".repeat(64)]),
    );
    assert.ok(displayed.every((row) => row.hasNameCollision));
    await act(async () => mention.handleMentionKeyDown(keyboard("ArrowDown")));
    const selected = displayed[1];
    state.searchUsers = [
      person("e".repeat(64)),
      person(OTHER),
      person("d".repeat(64)),
    ];
    await act(async () =>
      client.invalidateQueries({ queryKey: ["user-search"] }),
    );
    await settle();
    assert.deepEqual(mention.suggestions, displayed);
    assert.equal(mention.mentionSelectedIndex, 1);
    let outcome, edit;
    await act(async () => {
      outcome = mention.handleMentionKeyDown(keyboard(selectionKey));
    });
    assert.deepEqual(outcome.suggestion, selected);
    await act(async () => {
      edit = captureInsert(mention.selectMention)(outcome.suggestion, 6);
    });
    assert.equal(
      mention.getDraftMentionRefs(edit.insertText)[0].pubkey,
      selected.pubkey,
    );
    assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), [
      selected.pubkey,
    ]);
    await act(async () => mention.updateMentionQuery("@Scou", 5));
    await settle();
    assert.equal(mention.suggestions.length, 3);
  });
}

test("text changes load a new request; superseded and closed responses cannot install rows", async () => {
  await setup();
  let releaseOld, releaseNew, releaseClosed;
  state.pendingSearch = {
    old: new Promise((resolve) => {
      releaseOld = resolve;
    }),
    new: new Promise((resolve) => {
      releaseNew = resolve;
    }),
    closed: new Promise((resolve) => {
      releaseClosed = resolve;
    }),
  };
  await act(async () => mention.updateMentionQuery("@old", 4));
  await settle();
  assert.equal(mention.isMentionLoading, true);
  assert.equal(
    mention.handleMentionKeyDown(keyboard("Tab")).suggestion,
    undefined,
  );
  await act(async () => mention.updateMentionQuery("@new", 4));
  await settle();
  await act(async () =>
    releaseNew({ users: [person(OTHER, "New")], next_cursor: null }),
  );
  await settle();
  const displayed = mention.suggestions;
  assert.equal(displayed[0].pubkey, OTHER);
  await act(async () =>
    releaseOld({ users: [person(VIEWER, "Old")], next_cursor: null }),
  );
  await settle();
  assert.deepEqual(mention.suggestions, displayed);
  await act(async () => mention.updateMentionQuery("@closed", 7));
  await settle();
  await act(async () => mention.cancelMentionAutocomplete());
  await act(async () =>
    releaseClosed({ users: [person(VIEWER, "Closed")], next_cursor: null }),
  );
  await settle();
  assert.equal(mention.isMentionOpen, false);
  assert.deepEqual(mention.suggestions, []);
});

test("leaving a completion and navigation discard choices; explicit reopen starts at zero", async () => {
  await setup({ visible: true, directoryVisible: true });
  const old = rows()[0];
  await act(async () => mention.updateMentionQuery("plain", 5));
  assert.equal(mention.isMentionOpen, false);
  await act(async () => {
    assert.equal(captureInsert(mention.selectMention)(old, 5).insertText, "");
  });
  await act(async () => mention.openMentionPicker(5));
  assert.equal(mention.mentionSelectedIndex, 0);
  assert.notEqual(rows()[0], old);
  state.channelId = "another-channel";
  await render();
  assert.equal(mention.isMentionOpen, false);
  assert.deepEqual(mention.suggestions, []);
});

test("Space completes an exact name but remains literal for partial and same-name choices", async () => {
  await setup({ searchUsers: [person(OTHER, "Alice")] });
  await act(async () => mention.updateMentionQuery("@Ali", 4));
  await settle();
  assert.equal(mention.handleMentionKeyDown(keyboard(" ")).handled, false);
  await act(async () => mention.updateMentionQuery("@Alice", 6));
  await settle();
  assert.equal(
    mention.handleMentionKeyDown(keyboard(" ")).suggestion.pubkey,
    OTHER,
  );
  state.searchUsers = [person(OTHER, "Alice"), person("e".repeat(64), "Alice")];
  await act(async () =>
    client.invalidateQueries({ queryKey: ["user-search"] }),
  );
  await act(async () => mention.updateMentionQuery("@ALICE", 6));
  await settle();
  assert.equal(mention.handleMentionKeyDown(keyboard(" ")).handled, false);
  assert.equal(
    mention.handleMentionKeyDown(keyboard("Tab")).suggestion.pubkey,
    mention.suggestions[0].pubkey,
  );
});

for (const condition of ["denied", "missing", "failed", "cold-failed"]) {
  test(`disabled ${condition} member rejects pointer, keyboard and new pin intent`, async () => {
    await setup({
      owner: OTHER,
      visible: true,
      directoryVisible: true,
      policy: condition === "denied" ? "owner-only" : "anyone",
      missingDirectory: condition === "missing",
      failDirectory: condition.endsWith("failed"),
      coldDirectory: condition === "cold-failed",
      searchUsers: [person(OTHER, "Remote Person")],
    });
    await act(async () => mention.updateMentionQuery("@Remote", 7));
    await settle();
    assert.equal(mention.isMentionLoading, false);
    const row = rows()[0];
    if (condition === "cold-failed") {
      assert.equal(row.action, "unavailable");
      assert.equal(typeof row.onRetry, "function");
      assert.equal(
        mention.suggestions.some((s) => s.pubkey === OTHER),
        false,
      );
    }
    assert.equal(mention.canSelectMention(row), false);
    await act(async () => {
      picker.selectMentionSuggestion(row);
      picker.toggleAlwaysAddressAgent(row);
      mention.handleMentionKeyDown(keyboard("ArrowDown"));
    });
    for (const key of ["Tab", "Enter", " "]) {
      let outcome;
      await act(async () => {
        outcome = mention.handleMentionKeyDown(keyboard(key));
      });
      assert.equal(outcome.suggestion, undefined);
    }
    assert.deepEqual(effects, []);
    assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
    assert.deepEqual(mention.knownNames, []);
  });
}

test("checking resolves and retry refreshes without moving the selected identity", async () => {
  await setup({
    owner: OTHER,
    visible: true,
    directoryVisible: true,
    missingDirectory: true,
  });
  const identity = rows()[0].pubkey;
  assert.equal(rows()[0].action, "checking");
  state.missingDirectory = false;
  await act(async () =>
    client.invalidateQueries({ queryKey: ["relay-agents"] }),
  );
  await settle();
  assert.equal(rows()[0].pubkey, identity);
  assert.equal(mention.mentionSelectedIndex, 0);
  assert.equal(rows()[0].action, "mention");
  assert.equal(mention.canSelectMention(rows()[0]), true);
  state.failDirectory = true;
  await act(async () =>
    client.invalidateQueries({ queryKey: ["relay-agents"] }),
  );
  await settle();
  assert.equal(rows()[0].action, "unavailable");
  assert.equal(mention.canSelectMention(rows()[0]), false);
  state.failDirectory = false;
  await act(async () => rows()[0].onRetry());
  await settle();
  assert.equal(rows()[0].pubkey, identity);
  assert.equal(rows()[0].action, "mention");
});

test("verification expiry never installs an unfinished people search", async () => {
  await setup();
  let release;
  state.pendingSearch = {
    slow: new Promise((resolve) => {
      release = resolve;
    }),
  };
  await act(async () => mention.updateMentionQuery("@slow", 5));
  await act(async () => new Promise((resolve) => setTimeout(resolve, 5100)));
  assert.equal(mention.isMentionLoading, true);
  assert.deepEqual(mention.suggestions, []);
  await act(async () =>
    release({ users: [person(OTHER, "Slow")], next_cursor: null }),
  );
  await settle();
  assert.equal(mention.isMentionLoading, false);
  assert.equal(mention.suggestions[0].pubkey, OTHER);
});

test("cold directory expiry waits for required people search before installing choices", async () => {
  let releaseDirectory, releaseSearch;
  await setup({
    heldDirectory: new Promise((resolve) => {
      releaseDirectory = resolve;
    }),
    pendingSearch: {
      slow: new Promise((resolve) => {
        releaseSearch = resolve;
      }),
    },
  });
  await act(async () => mention.updateMentionQuery("@Slow", 5));
  await act(async () => new Promise((resolve) => setTimeout(resolve, 5100)));
  assert.equal(
    mention.isMentionLoading,
    true,
    "directory expiry cannot complete a not-yet-enabled search",
  );
  assert.deepEqual(mention.suggestions, []);
  state.heldDirectory = null;
  await act(async () => releaseDirectory([]));
  await settle();
  assert.equal(
    mention.isMentionLoading,
    true,
    "enabling search is not settlement",
  );
  assert.deepEqual(mention.suggestions, []);
  await act(async () =>
    releaseSearch({ users: [person(OTHER, "Slow Person")], next_cursor: null }),
  );
  await settle();
  assert.equal(mention.isMentionLoading, false);
  assert.deepEqual(
    mention.suggestions.map((row) => row.pubkey),
    [OTHER],
  );
  assert.equal(
    mention.handleMentionKeyDown(keyboard("Tab")).suggestion.pubkey,
    OTHER,
  );
});

for (const visible of [true, false]) {
  test(`cached allowed ${visible ? "member" : "relay-only nonmember"} expiry blocks every retained choice until fresh retry settles`, async () => {
    await setup({
      owner: OTHER,
      visible,
      directoryVisible: true,
      policy: "anyone",
    });
    await act(async () => mention.updateMentionQuery("@Remote", 7));
    await settle();
    const retained = rows()[0];
    assert.equal(retained.action, visible ? "mention" : "invite");
    const identities = mention.suggestions.map((row) => [
      row.pubkey,
      row.personaId,
      row.teamId,
      row.displayName,
    ]);
    await act(async () => new Promise((resolve) => setTimeout(resolve, 5100)));
    assert.equal(rows()[0].pubkey, retained.pubkey);
    assert.equal(rows()[0].action, "unavailable");
    assert.equal(
      rows()[0].unavailableReason,
      "Could not verify access. Retry to check again.",
    );
    const assertBlocked = async () => {
      assert.equal(mention.canSelectMention(retained), false);
      await act(async () => {
        picker.selectMentionSuggestion(retained);
        picker.toggleAlwaysAddressAgent(retained);
      });
      for (const key of ["Tab", "Enter", " "]) {
        let outcome;
        await act(async () => {
          outcome = mention.handleMentionKeyDown(keyboard(key));
        });
        assert.equal(outcome.suggestion, undefined);
      }
      assert.deepEqual(effects, []);
      assert.deepEqual(mention.knownNames, []);
    };
    await assertBlocked();
    let release;
    state.heldDirectory = new Promise((resolve) => {
      release = resolve;
    });
    await act(async () => rows()[0].onRetry());
    assert.equal(rows()[0].action, "checking");
    await assertBlocked();
    state.heldDirectory = null;
    await act(async () => release([rawAgent()]));
    await settle();
    assert.equal(rows()[0].action, visible ? "mention" : "invite");
    assert.deepEqual(
      mention.suggestions.map((row) => [
        row.pubkey,
        row.personaId,
        row.teamId,
        row.displayName,
      ]),
      identities,
    );
    assert.equal(rows()[0].pubkey, retained.pubkey);
    assert.equal(mention.mentionSelectedIndex, 0);
    assert.equal(mention.canSelectMention(retained), true);
  });
}

for (const failure of ["denied", "lookup-failed"]) {
  test(`installed relay-only nonmember preserves ${failure} reason and held Retry`, async () => {
    await setup({ owner: OTHER, visible: false, directoryVisible: true });
    const action = "invite";
    const retained = rows()[0];
    assert.equal(retained.action, action);
    const identities = mention.suggestions.map((row) => [
      row.pubkey,
      row.personaId,
      row.teamId,
      row.displayName,
    ]);
    state.policy = failure === "denied" ? "owner-only" : "anyone";
    state.failDirectory = failure === "lookup-failed";
    await act(async () =>
      client.invalidateQueries({ queryKey: ["relay-agents"] }),
    );
    await settle();
    assert.equal(rows()[0].action, "unavailable");
    assert.equal(
      rows()[0].unavailableReason,
      failure === "denied"
        ? "This agent does not permit you to mention it here."
        : "Could not verify access. Retry to check again.",
    );
    assert.equal(mention.canSelectMention(retained), false);
    let release, reject;
    state.heldDirectory = new Promise((resolve, fail) => {
      release = resolve;
      reject = fail;
    });
    await act(async () => rows()[0].onRetry());
    assert.equal(rows()[0].action, "checking");
    assert.equal(rows()[0].onRetry, undefined);
    assert.equal(mention.canSelectMention(retained), false);
    if (failure === "lookup-failed") {
      await act(async () => reject(new Error("Retry failed")));
      await settle();
      assert.equal(rows()[0].action, "unavailable");
      assert.equal(
        rows()[0].unavailableReason,
        "Could not verify access. Retry to check again.",
      );
      assert.equal(mention.canSelectMention(retained), false);
      state.heldDirectory = new Promise((resolve) => {
        release = resolve;
      });
      await act(async () => rows()[0].onRetry());
      assert.equal(rows()[0].action, "checking");
    }
    state.policy = "anyone";
    state.failDirectory = false;
    state.heldDirectory = null;
    await act(async () => release([rawAgent()]));
    await settle();
    assert.equal(rows()[0].action, action);
    assert.equal(mention.canSelectMention(rows()[0]), true);
    assert.deepEqual(
      mention.suggestions.map((row) => [
        row.pubkey,
        row.personaId,
        row.teamId,
        row.displayName,
      ]),
      identities,
    );
    assert.equal(mention.mentionSelectedIndex, 0);
    state.policy = "owner-only";
    await act(async () =>
      client.invalidateQueries({ queryKey: ["relay-agents"] }),
    );
    await settle();
    await act(async () => mention.updateMentionQuery("@Remote", 7));
    await settle();
    assert.deepEqual(
      rows(),
      [],
      "fresh discovery must still exclude denied nonmembers",
    );
  });
}
// Observe the real admitted action after act flushes its promise; no authority stub.
function captureInsert(select) {
  return (row, cursor) => {
    const edit = {
      insertText: "",
      replaceFromOffset: cursor,
      replaceToOffset: cursor,
    };
    select(
      row,
      cursor,
      () => true,
      (committed) => Object.assign(edit, committed),
    );
    return edit;
  };
}
