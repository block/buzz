import { closeHistory } from "@tiptap/pm/history";
import {
  getMentionSelectionHistory,
  resetMentionSelectionHistory,
} from "../messages/lib/mentionSelectionHistory.ts";
// Production mention + CHAT picker + native Tiptap boundary; only IPC is fixture evidence.
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
// JSDOM has no layout; geometry is not an admission or input-dispatch fixture.
dom.window.HTMLElement.prototype.scrollIntoView = () => {};
dom.window.Range.prototype.getClientRects = () => [];
dom.window.Range.prototype.getBoundingClientRect = () =>
  new dom.window.DOMRect();
// Radix tooltip focus-open runs document.dispatchEvent(new
// CustomEvent(TOOLTIP_OPEN)) against the ambient global; Node's CustomEvent is a
// foreign realm to this jsdom document, so install the jsdom constructor and
// restore Node's original in the after() teardown.
const originalCustomEvent = globalThis.CustomEvent;
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  CustomEvent: dom.window.CustomEvent,
  localStorage: dom.window.localStorage,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  Node: dom.window.Node,
  getComputedStyle: dom.window.getComputedStyle,
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
dom.window.cancelAnimationFrame = clearTimeout;
globalThis.cancelAnimationFrame = clearTimeout;
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
  if (command === "revalidate_relay_agents") {
    state.freshCalls = (state.freshCalls ?? 0) + 1;
    assert.deepEqual(args.pubkeys, [AGENT]);
    assert.equal(args.channelId, state.channelId);
    if (state.fresh) return state.fresh;
    if (state.failFresh) throw new Error("offline");
    return state.missingDirectory ? [] : [rawAgent()];
  }
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
let useMentions, useRichTextEditor, EditorContent, richText;
let root, client, mention, picker, focusMentionOptionsTrigger;
let useAgentAddressLockPicker, effects, MentionAutocomplete, TooltipProvider;
before(async () => {
  ({ MentionAutocomplete, focusMentionOptionsTrigger } = await import(
    "@/features/messages/ui/MentionAutocomplete.tsx"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
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
  ({ useRichTextEditor } = await import(
    "@/features/messages/lib/useRichTextEditor.ts"
  ));
  ({ EditorContent } = await import("@tiptap/react"));
  ({ useMentions } = await import("@/features/messages/lib/useMentions.ts"));
});
function Composer() {
  const open = React.useRef(false);
  const formRef = React.useRef(null);
  const [keepPinned, setKeepPinned] = React.useState(true);
  richText = useRichTextEditor({
    isAutocompleteOpen: open,
    onSubmit: () => effects.push(["submit"]),
    onUpdate: ({ text, cursor }) => mention?.updateMentionQuery(text, cursor),
    onSelectionUpdate: ({ text, cursor }) =>
      mention?.updateMentionQuery(text, cursor),
  });
  mention = useMentions(state.channelId, undefined, undefined, {
    channelType: "stream",
    getEditorSnapshot: richText.getPlainTextAndCursor,
  });
  open.current = mention.isMentionOpen;
  picker = useAgentAddressLockPicker({
    mentions: mention,
    audience: {
      pubkeys: state.locked,
      addPubkey: (key) => effects.push(["pin", key]),
      removePubkey: (key) => effects.push(["remove", key]),
    },
    audienceScope: state.channelId,
    richText,
    applyAutocompleteEdit: (edit) => {
      effects.push(["edit", edit]);
      richText.replacePlainTextRange(
        edit.replaceFromOffset,
        edit.replaceToOffset,
        edit.insertText,
        undefined,
        edit.preserveSelection,
        edit.reassertMentionCaret,
      );
    },
    onAddressAgentMention: (row) => effects.push(["promote", row.pubkey]),
    onAutoPinAgentMention: (row) => effects.push(["autoPin", row.pubkey]),
    onImplicitPrefixInserted: (refs) => effects.push(["provenance", refs]),
    onPulseAddressLock: () => effects.push(["pulse"]),
  });
  return React.createElement(
    "div",
    { ref: formRef },
    React.createElement(
      "div",
      {
        onKeyDown: (event) => {
          // Only editor events reach this bridge, as in MessageComposer.
          if (
            event.key === "Tab" &&
            event.shiftKey &&
            event.target === richText.editor.view.dom &&
            focusMentionOptionsTrigger(formRef.current)
          ) {
            event.preventDefault();
            return;
          }
          const result = mention.handleMentionKeyDown(event);
          if (result.suggestion)
            picker.selectMentionSuggestion(result.suggestion);
        },
      },
      React.createElement(EditorContent, { editor: richText.editor }),
    ),
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(MentionAutocomplete, {
        composerOwnsFocus: true,
        keepMentionedAgentsPinned: keepPinned,
        onKeepMentionedAgentsPinnedChange: state.withOptions
          ? setKeepPinned
          : undefined,
        isOpen: mention.isMentionOpen,
        isLoading: mention.isMentionLoading,
        suggestions: mention.suggestions,
        selectedIndex: mention.mentionSelectedIndex,
        onSelect: picker.selectMentionSuggestion,
        onToggleAlwaysAddressAgent: picker.toggleAlwaysAddressAgent,
        alwaysAddressedAgentPubkeys: new Set(state.locked),
      }),
    ),
  );
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
    withOptions: true,
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
  await act(async () => {
    richText.setContent("@");
    richText.editor.commands.setTextSelection(2);
    mention.updateMentionQuery("@", 1);
  });
  await settle();
  for (
    let i = 0;
    i < mention.suggestions.length &&
    mention.suggestions[mention.mentionSelectedIndex]?.pubkey !== AGENT;
    i++
  ) {
    await act(async () => choose("ArrowDown"));
  }
  assert.equal(
    mention.suggestions[mention.mentionSelectedIndex]?.pubkey,
    AGENT,
  );
}
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  resetMentionSelectionHistory();
  client?.clear();
  document.body.replaceChildren();
});
after(() => {
  globalThis.CustomEvent = originalCustomEvent;
  dom.window.close();
});

function unchanged() {
  assert.equal(richText.getPlainTextAndCursor().text, "@");
  assert.deepEqual(mention.getDraftMentionRefs("@Remote Scout "), []);
  assert.deepEqual(mention.knownNames, []);
  assert.deepEqual(mention.agentKnownNames, []);
  assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
  assert.deepEqual(effects, []);
}
function choose(mode) {
  if (mode === "pin") {
    const toggle = document.querySelector(
      `[data-testid="mention-always-address-${AGENT}"]`,
    );
    assert.ok(toggle, "production pin Toggle");
    toggle.click();
  } else if (mode === "pointer") picker.selectMentionSuggestion(rows()[0]);
  else {
    const event = new dom.window.KeyboardEvent("keydown", {
      key: mode,
      bubbles: true,
      cancelable: true,
    });
    richText.editor.view.dom.dispatchEvent(event);
    assert.equal(event.defaultPrevented, true);
  }
}
for (const mode of ["pointer", "Enter", "Tab", "pin"]) {
  for (const outcome of ["allow", "revoke", "failure"]) {
    test(`${mode}: fresh ${outcome} is atomic without discovery refresh`, async () => {
      await setup({ owner: OTHER, visible: true, directoryVisible: true });
      let resolve;
      state.fresh = new Promise((r) => {
        resolve = r;
      });
      await act(async () => choose(mode));
      assert.match(picker.announcement, /Checking/);
      unchanged();
      assert.equal(state.freshCalls, 1);
      if (outcome === "revoke") state.policy = "owner-only";
      await act(async () =>
        resolve(
          outcome === "failure"
            ? Promise.reject(new Error("offline"))
            : [rawAgent()],
        ),
      );
      await settle();
      if (outcome === "allow") {
        assert.equal(richText.getPlainTextAndCursor().text, "@Remote Scout ");
        assert.equal(
          effects.filter(([kind]) => kind === "edit").length,
          mode === "pin" ? 2 : 1,
        );
        assert.equal(
          effects.filter(
            ([kind]) => kind === (mode === "pin" ? "promote" : "autoPin"),
          ).length,
          1,
        );
        assert.equal(
          mention.getDraftMentionRefs("@Remote Scout ")[0].pubkey,
          AGENT,
        );
        assert.equal(
          getMentionSelectionHistory(VIEWER, CHANNEL).length,
          mode === "pin" ? 0 : 1,
        );
      } else {
        unchanged();
        assert.match(
          picker.announcement,
          outcome === "revoke" ? /Access changed/ : /Could not check/,
        );
      }
    });
  }
}
for (const change of [
  "edit-undo",
  "caret-return",
  "scope-return",
  "unmount",
  "Escape",
  "ArrowDown",
]) {
  test(`late allow after ${change} cannot commit`, async () => {
    await setup({ owner: OTHER, visible: true, directoryVisible: true });
    let resolve;
    state.fresh = new Promise((r) => {
      resolve = r;
    });
    await act(async () => choose("Enter"));
    unchanged();
    if (change === "scope-return") {
      state.channelId = "other";
      await render();
      state.channelId = CHANNEL;
      await render();
    } else if (change === "unmount") await render(false);
    else
      await act(async () => {
        if (change === "edit-undo") {
          richText.editor.view.dispatch(closeHistory(richText.editor.state.tr));
          richText.editor.commands.insertContent("x");
          richText.editor.commands.undo();
          assert.equal(richText.getPlainTextAndCursor().text, "@");
        } else if (change === "caret-return") {
          richText.editor.commands.setTextSelection(1);
          richText.editor.commands.setTextSelection(2);
        } else choose(change);
      });
    await act(async () => resolve([rawAgent()]));
    await settle();
    assert.deepEqual(effects, []);
    assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
    if (change !== "unmount")
      assert.doesNotMatch(picker.announcement, /Checking/);
  });
}

// JSDOM does not perform native Tab movement or keyboard-generated clicks.
// Dispatch the key, then enact that browser default explicitly; the focus and
// production admission/picker/editor effects are real, not mocked cancellation.
for (const mode of ["Enter", "pin"]) {
  test(`navigation: editor ShiftTab abandons ${mode} even after return`, async () => {
    await setup({ owner: OTHER, visible: true, directoryVisible: true });
    let resolve;
    state.fresh = new Promise((r) => {
      resolve = r;
    });
    await act(async () => {
      richText.editor.view.dom.focus();
      choose(mode);
    });
    unchanged();
    await act(async () => {
      richText.editor.view.dom.dispatchEvent(
        new dom.window.KeyboardEvent("keydown", {
          key: "Tab",
          shiftKey: true,
          bubbles: true,
          cancelable: true,
        }),
      );
      const trigger = document.querySelector("[data-mention-options-trigger]");
      assert.ok(trigger, "chat Options trigger exists");
      assert.equal(
        document.activeElement === trigger,
        true,
        "ShiftTab focuses Options",
      );
      richText.editor.view.dom.focus();
    });
    await act(async () => resolve([rawAgent()]));
    await settle();
    unchanged();
    assert.equal(picker.announcement, "");
  });
}
for (const shiftKey of [false, true]) {
  for (const depart of [false, true]) {
    test(`navigation: focused pin ${shiftKey ? "ShiftTab" : "Tab"} depart=${depart}`, async () => {
      await setup({ owner: OTHER, visible: true, directoryVisible: true });
      let resolve;
      state.fresh = new Promise((r) => {
        resolve = r;
      });
      const toggle = document.querySelector(
        `[data-testid="mention-always-address-${AGENT}"]`,
      );
      assert.ok(toggle, "production pin Toggle exists");
      await act(async () => {
        toggle.focus();
        assert.equal(
          document.activeElement === toggle,
          true,
          "keyboard pin owns focus",
        );
        const key = shiftKey ? " " : "Enter";
        const event = new dom.window.KeyboardEvent("keydown", {
          key,
          bubbles: true,
          cancelable: true,
        });
        toggle.dispatchEvent(event);
        assert.equal(
          event.defaultPrevented,
          false,
          "overlay activation stays native",
        );
        assert.equal(
          state.freshCalls ?? 0,
          0,
          "pin keydown must not select in editor",
        );
        const release = new dom.window.KeyboardEvent("keyup", {
          key,
          bubbles: true,
          cancelable: true,
        });
        // JSDOM lacks native activation: Enter clicks on keydown, Space on keyup.
        if (key === " ") toggle.dispatchEvent(release);
        if (!event.defaultPrevented && !release.defaultPrevented)
          toggle.click();
      });
      unchanged();
      assert.equal(state.freshCalls, 1);
      if (depart)
        await act(async () => {
          const event = new dom.window.KeyboardEvent("keydown", {
            key: "Tab",
            shiftKey,
            bubbles: true,
            cancelable: true,
          });
          toggle.dispatchEvent(event);
          assert.equal(
            event.defaultPrevented,
            false,
            "overlay Tab stays native",
          );
          const outside = document.createElement("button");
          document.body.append(outside);
          if (!event.defaultPrevented) outside.focus();
          assert.equal(
            document.activeElement === outside,
            true,
            "Tab departs pin",
          );
          toggle.focus();
        });
      await act(async () => resolve([rawAgent()]));
      await settle();
      if (depart) {
        unchanged();
        assert.equal(picker.announcement, "");
      } else {
        assert.deepEqual(
          effects.filter(([kind]) => kind === "promote"),
          [["promote", AGENT]],
        );
        assert.deepEqual(
          effects.filter(([kind]) => kind === "autoPin"),
          [],
        );
        assert.deepEqual(
          effects.filter(([kind]) => kind === "provenance"),
          [["provenance", [{ pubkey: AGENT, prefix: "@Remote Scout " }]]],
        );
        assert.equal(effects.filter(([kind]) => kind === "edit").length, 2);
        assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
        assert.equal(richText.getPlainTextAndCursor().text, "@Remote Scout ");
        assert.equal(
          mention.getDraftMentionRefs("@Remote Scout ")[0]?.pubkey,
          AGENT,
        );
        // MentionHighlightExtension decorates literal @labels from either path;
        // .mention-chip is not selection provenance. The pin witnesses above
        // distinguish admission paths; verify the prefix is plain document text
        // (useRichTextEditor.replacePlainTextRange), not an embedded mention node.
        assert.deepEqual(richText.editor.getJSON(), {
          type: "doc",
          content: [
            {
              type: "paragraph",
              content: [{ type: "text", text: "@Remote Scout " }],
            },
          ],
        });
      }
    });
  }
}

for (const departure of ["window", "no-Options native fallback"]) {
  test(`navigation: ${departure} abandons pending selection after return`, async () => {
    await setup({
      owner: OTHER,
      visible: true,
      directoryVisible: true,
      withOptions: departure !== "no-Options native fallback",
    });
    let resolve;
    state.fresh = new Promise((r) => {
      resolve = r;
    });
    await act(async () => {
      richText.editor.view.dom.focus();
      choose("Enter");
    });
    unchanged();
    assert.equal(state.freshCalls, 1);
    await act(async () => {
      const editor = richText.editor.view.dom;
      if (departure === "window") {
        // Window departure can retain activeElement; dispatch only that boundary.
        dom.window.dispatchEvent(new dom.window.Event("blur"));
        assert.equal(
          document.activeElement === editor,
          true,
          "window blur retains editor identity",
        );
        dom.window.dispatchEvent(new dom.window.Event("focus"));
      } else {
        assert.equal(
          document.querySelector("[data-mention-options-trigger]") === null,
          true,
        );
        const event = new dom.window.KeyboardEvent("keydown", {
          key: "Tab",
          shiftKey: true,
          bubbles: true,
          cancelable: true,
        });
        editor.dispatchEvent(event);
        assert.equal(
          event.defaultPrevented,
          false,
          "no Options leaves ShiftTab native",
        );
        // Explicit native Tab default emulation, not a browser tab-order claim.
        const outside = document.createElement("button");
        document.body.append(outside);
        if (!event.defaultPrevented) outside.focus();
        assert.equal(
          document.activeElement === outside,
          true,
          "native fallback departs editor",
        );
        editor.focus();
      }
    });
    await act(async () => resolve([rawAgent()]));
    await settle();
    unchanged();
    assert.equal(picker.announcement, "");
  });
}

test("literal Space remains native, normal Enter submits outside chooser", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  const event = new dom.window.KeyboardEvent("keydown", {
    key: " ",
    bubbles: true,
    cancelable: true,
  });
  await act(async () => richText.editor.view.dom.dispatchEvent(event));
  assert.equal(event.defaultPrevented, false);
  assert.equal(state.freshCalls ?? 0, 0);
  await act(async () => mention.cancelMentionAutocomplete());
  await act(async () => choose("Enter"));
  assert.deepEqual(effects, [["submit"]]);
});
test("repeat Enter while pending consumes input without another lookup", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  let resolve;
  state.fresh = new Promise((r) => {
    resolve = r;
  });
  await act(async () => choose("Enter"));
  await act(async () => choose("Enter"));
  unchanged();
  assert.equal(state.freshCalls, 1);
  await act(async () => resolve([rawAgent()]));
  await settle();
  assert.equal(effects.filter(([kind]) => kind === "edit").length, 1);
});

test("exact Space enters the same fresh operation", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  await act(async () => {
    richText.setContent("@Remote Scout");
    richText.editor.commands.setTextSelection(14);
    mention.updateMentionQuery("@Remote Scout", 13);
  });
  await settle();
  let resolve;
  state.fresh = new Promise((r) => {
    resolve = r;
  });
  await act(async () => choose(" "));
  assert.match(picker.announcement, /Checking/);
  assert.equal(richText.getPlainTextAndCursor().text, "@Remote Scout");
  assert.deepEqual(effects, []);
  assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
  assert.equal(state.freshCalls, 1);
  await act(async () => resolve([rawAgent()]));
  await settle();
  assert.equal(richText.getPlainTextAndCursor().text, "@Remote Scout ");
});
test("late lookup rejection after native input cancellation is silent", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  let reject;
  state.fresh = new Promise((_resolve, r) => {
    reject = r;
  });
  await act(async () => choose("Enter"));
  await act(async () =>
    richText.editor.view.dom.dispatchEvent(
      new dom.window.InputEvent("beforeinput", {
        bubbles: true,
        inputType: "insertText",
        data: "x",
      }),
    ),
  );
  await act(async () => reject(new Error("late offline")));
  await settle();
  unchanged();
  assert.equal(picker.announcement, "");
});

for (const change of ["edit-undo", "unpin", "failure-retry"]) {
  test(`pin pending: ${change}`, async () => {
    await setup({ owner: OTHER, visible: true, directoryVisible: true });
    let resolve;
    state.fresh = new Promise((r) => {
      resolve = r;
    });
    await act(async () => choose("pin"));
    unchanged();
    if (change === "edit-undo") {
      await act(async () => {
        richText.editor.view.dispatch(closeHistory(richText.editor.state.tr));
        richText.editor.commands.insertContent("x");
        richText.editor.commands.undo();
        assert.equal(richText.getPlainTextAndCursor().text, "@");
      });
    } else if (change === "unpin") {
      state.locked = [AGENT];
      await render();
      await act(async () => picker.toggleAlwaysAddressAgent(rows()[0]));
      assert.ok(effects.some(([kind]) => kind === "remove"));
      effects = [];
    }
    await act(async () =>
      resolve(
        change === "failure-retry"
          ? Promise.reject(new Error("offline"))
          : [rawAgent()],
      ),
    );
    await settle();
    assert.deepEqual(effects, []);
    assert.deepEqual(mention.knownNames, []);
    assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
    if (change === "failure-retry") {
      unchanged();
      state.fresh = Promise.resolve([rawAgent()]);
      await act(async () => choose("pin"));
      await settle();
      assert.equal(state.freshCalls, 2);
      assert.ok(effects.some(([kind]) => kind === "promote"));
    }
  });
}

async function setupForum(text = "@") {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  const { ForumComposer } = await import(
    "@/features/forum/ui/ForumComposer.tsx"
  );
  const { createRouter, createRootRoute, createMemoryHistory, RouterProvider } =
    await import("@tanstack/react-router");
  const { TooltipProvider } = await import("@/shared/ui/tooltip.tsx");
  const route = createRootRoute({
    component: () =>
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(ForumComposer, {
          channelId: CHANNEL,
          channelType: "forum",
          onSubmit: async (...args) => effects.push(["submit", ...args]),
        }),
      ),
  });
  const router = createRouter({
    routeTree: route,
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  await act(async () =>
    root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(RouterProvider, { router }),
        ),
      ),
    ),
  );
  await settle();
  const element = document.querySelector(".tiptap");
  const editor = element.editor;
  assert.ok(editor, "actual standalone ForumComposer Tiptap editor");
  await act(async () => {
    editor.commands.setContent(text);
    editor.commands.setTextSelection(text.length + 1);
    editor.view.focus();
  });
  await settle();
  return editor;
}
for (const mode of ["pointer", "Enter", "Tab", " "])
  for (const outcome of ["allow", "revoke", "failure", "cancel"]) {
    test(`standalone forum production component ${mode}: ${outcome}`, async () => {
      const draft = mode === " " ? "@Remote Scout" : "@";
      const editor = await setupForum(draft);
      let resolve;
      state.fresh = new Promise((r) => {
        resolve = r;
      });
      const row = document.querySelector(
        `[data-testid="mention-suggestion-${AGENT}"]`,
      );
      assert.ok(row, "displayed exact agent row");
      await act(async () => {
        if (mode === "pointer")
          row.querySelector("button").dispatchEvent(
            new dom.window.MouseEvent("mousedown", {
              bubbles: true,
              cancelable: true,
            }),
          );
        else {
          for (let i = 0; i < Number(row.dataset.mentionSuggestionIndex); i++)
            editor.view.dom.dispatchEvent(
              new dom.window.KeyboardEvent("keydown", {
                key: "ArrowDown",
                bubbles: true,
                cancelable: true,
              }),
            );
        }
      });
      if (mode !== "pointer")
        await act(async () =>
          editor.view.dom.dispatchEvent(
            new dom.window.KeyboardEvent("keydown", {
              key: mode,
              bubbles: true,
              cancelable: true,
            }),
          ),
        );
      assert.equal(editor.getText(), draft);
      assert.match(document.querySelector("output").textContent, /Checking/);
      assert.deepEqual(getMentionSelectionHistory(VIEWER, CHANNEL), []);
      if (outcome === "revoke") state.policy = "owner-only";
      if (outcome === "cancel")
        await act(async () => {
          editor.commands.setTextSelection(1);
          editor.commands.setTextSelection(2);
        });
      await act(async () =>
        resolve(
          outcome === "failure"
            ? Promise.reject(new Error("offline"))
            : [rawAgent()],
        ),
      );
      await settle();
      assert.equal(
        editor.getText(),
        outcome === "allow" ? "@Remote Scout " : draft,
      );
      assert.equal(
        getMentionSelectionHistory(VIEWER, CHANNEL).length,
        outcome === "allow" ? 1 : 0,
      );
      assert.deepEqual(effects, []);
      if (outcome === "revoke" || outcome === "failure") {
        assert.match(
          document.querySelector("output").textContent,
          outcome === "revoke" ? /Access changed/ : /Could not check/,
        );
        state.policy = "anyone";
        state.fresh = Promise.resolve([rawAgent()]);
        await act(async () =>
          row.querySelector("button").dispatchEvent(
            new dom.window.MouseEvent("mousedown", {
              bubbles: true,
              cancelable: true,
            }),
          ),
        );
        await settle();
        assert.equal(editor.getText(), "@Remote Scout ");
        assert.equal(state.freshCalls, 2);
      }
      if (outcome === "allow") {
        state.policy = "owner-only";
        state.fresh = Promise.resolve([rawAgent()]);
        await act(async () =>
          document.querySelector("form").dispatchEvent(
            new dom.window.Event("submit", {
              bubbles: true,
              cancelable: true,
            }),
          ),
        );
        await settle();
        assert.equal(editor.getText(), "@Remote Scout ");
        assert.equal(
          state.freshCalls,
          2,
          "publication must check authority independently",
        );
        assert.deepEqual(effects, []);
      }
    });
  }

test("pin authority timeout is retryable and late allow cannot mutate", async () => {
  await setup({ owner: OTHER, visible: true, directoryVisible: true });
  let resolve;
  state.fresh = new Promise((r) => {
    resolve = r;
  });
  await act(async () => choose("pin"));
  unchanged();
  await act(async () => new Promise((r) => setTimeout(r, 15100)));
  assert.match(picker.announcement, /Could not check/);
  unchanged();
  await act(async () => resolve([rawAgent()]));
  await settle();
  unchanged();
  state.fresh = Promise.resolve([rawAgent()]);
  await act(async () => choose("pin"));
  await settle();
  assert.ok(effects.some(([kind]) => kind === "promote"));
});

for (const outcome of ["allow", "revoke", "failure", "cancel"]) {
  test(`closed-picker default pin: ${outcome}`, async () => {
    await setup({ owner: OTHER, visible: true, directoryVisible: true });
    await act(async () => {
      richText.setContent("");
      mention.cancelMentionAutocomplete();
    });
    await settle();
    const row = mention.getDefaultAgentSuggestion();
    assert.equal(row.pubkey, AGENT);
    let resolve;
    state.fresh = new Promise((r) => {
      resolve = r;
    });
    await act(async () => picker.toggleAlwaysAddressAgent(row));
    assert.match(picker.announcement, /Checking/);
    assert.deepEqual(effects, []);
    if (outcome === "revoke") state.policy = "owner-only";
    if (outcome === "cancel")
      await act(async () =>
        richText.editor.view.dom.dispatchEvent(
          new dom.window.InputEvent("beforeinput", { bubbles: true }),
        ),
      );
    await act(async () =>
      resolve(
        outcome === "failure"
          ? Promise.reject(new Error("offline"))
          : [rawAgent()],
      ),
    );
    await settle();
    assert.equal(
      richText.getPlainTextAndCursor().text,
      outcome === "allow" ? "@Remote Scout " : "",
    );
    if (outcome !== "allow") {
      assert.deepEqual(effects, []);
      assert.deepEqual(mention.knownNames, []);
    }
  });
}

test("standalone forum literal Space and Enter outside chooser retain native dispatch", async () => {
  const editor = await setupForum("@Rem");
  const space = new dom.window.KeyboardEvent("keydown", {
    key: " ",
    bubbles: true,
    cancelable: true,
  });
  await act(async () => editor.view.dom.dispatchEvent(space));
  assert.equal(space.defaultPrevented, false);
  assert.equal(state.freshCalls, undefined);
  await act(async () => {
    editor.commands.setContent("ordinary text");
    editor.commands.setTextSelection(14);
  });
  await settle();
  await act(async () =>
    editor.view.dom.dispatchEvent(
      new dom.window.KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  await settle();
  assert.equal(effects.filter(([kind]) => kind === "submit").length, 1);
});
