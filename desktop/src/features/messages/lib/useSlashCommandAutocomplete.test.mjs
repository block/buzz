import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import * as React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { JSDOM } from "jsdom";
import {
  initAgentCommandCatalog,
  recordAvailableCommandsUpdate,
  resetAgentCommandCatalogForTests,
} from "@/features/agents/agentCommandCatalog.ts";
import {
  extractMentionPubkeys,
  selectedMentionLabel,
} from "./extractMentionPubkeys.ts";
import { useSlashCommandAutocomplete } from "./useSlashCommandAutocomplete.ts";

const OWNER = "a".repeat(64);
const ALPHA = "b".repeat(64);
const BETA = "c".repeat(64);
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const clients = [];

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});
afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  for (const client of clients.splice(0)) client.clear();
  resetAgentCommandCatalogForTests();
  window.localStorage.clear();
});
after(() => dom.window.close());

async function setup({ sameName = false } = {}) {
  initAgentCommandCatalog("test-community");
  const { renderHook } = await import("@testing-library/react");
  const members = [
    { pubkey: ALPHA, displayName: "Alpha", isAgent: true, isMember: true },
    {
      pubkey: BETA,
      displayName: sameName ? "Alpha" : "Beta",
      isAgent: true,
      isMember: true,
    },
  ];
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  clients.push(client);
  client.setQueryData(["channels", "channel", "members"], members);
  client.setQueryData(["channels", "other-channel", "members"], members);
  for (const member of members) {
    recordAvailableCommandsUpdate(OWNER, member.pubkey, {
      seq: 1,
      timestamp: "2026-07-23T08:00:00Z",
      payload: { commands: [{ name: "review" }, { name: "deploy" }] },
    });
  }
  const bindings = new Map();
  const mentions = {
    registerMentionPubkey(name, pubkey) {
      const label = selectedMentionLabel(name, pubkey, bindings);
      bindings.set(label, pubkey);
      return label;
    },
    getMentionIdentities: () => [
      ...[...bindings].map(([label, pubkey]) => ({
        label,
        pubkey,
        isAgent: true,
      })),
      ...members
        .filter((member) => !bindings.has(member.displayName))
        .map((member) => ({
          label: member.displayName,
          pubkey: member.pubkey,
          isAgent: true,
        })),
    ],
    extractMentionPubkeys: (text) =>
      extractMentionPubkeys({
        text,
        selectedMentions: bindings,
        memberCandidates: members,
      }),
  };
  const hook = renderHook(
    ({ channelId }) =>
      useSlashCommandAutocomplete({
        channelId,
        ownerPubkey: OWNER,
        mentions,
      }),
    {
      initialProps: { channelId: "channel" },
      wrapper: ({ children }) =>
        React.createElement(QueryClientProvider, { client }, children),
    },
  );
  return { ...hook, bindings, mentions };
}

function keyEvent(key, modifiers = {}) {
  return {
    key,
    preventDefault() {
      this.defaultPrevented = true;
    },
    ...modifiers,
  };
}

test("keyboard selection inserts and binds the exact registered same-name agent label", async () => {
  const { act } = await import("@testing-library/react");
  const { result, mentions } = await setup({ sameName: true });
  mentions.registerMentionPubkey("Alpha", ALPHA);
  act(() => result.current.updateQuery("/rev", 4));
  act(() => result.current.handleKeyDown(keyEvent("ArrowDown")));
  let edit;
  act(() => {
    const selection = result.current.handleKeyDown(keyEvent("Enter"));
    edit = result.current.insertCommand(selection.suggestion, 4);
  });
  assert.equal(edit.insertText, `@Alpha (${BETA}) /review `);
  assert.deepEqual(mentions.extractMentionPubkeys(edit.insertText), [BETA]);
  assert.equal(result.current.isOpen, false);
  const text = `@Alpha (${BETA}) /rev`;
  act(() => result.current.updateQuery(text, text.length));
  assert.deepEqual(
    result.current.groups.map((group) => group.agentPubkey),
    [BETA],
  );
});

test("ambiguous manually typed names do not offer a command to an arbitrary agent", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await setup({ sameName: true });
  act(() => result.current.updateQuery("@Alpha /rev", 11));
  assert.equal(result.current.isOpen, false);
});

test("escape keeps the literal query dismissed until it changes and tab selects", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await setup();
  act(() => result.current.updateQuery("/", 1));
  act(() => result.current.handleKeyDown(keyEvent("Escape")));
  act(() => result.current.updateQuery("/", 1));
  assert.equal(result.current.isOpen, false);
  act(() => result.current.updateQuery("/rev", 4));
  assert.equal(result.current.isOpen, true);
  assert.equal(
    result.current.handleKeyDown(keyEvent("Enter", { shiftKey: true })).handled,
    false,
  );
  assert.equal(
    result.current.handleKeyDown(keyEvent("Tab")).suggestion.name,
    "review",
  );
});

test("live catalog removal closes the picker without clearing a leading mention", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await setup();
  act(() => result.current.updateQuery("@Alpha /rev", 11));
  assert.equal(result.current.isOpen, true);
  act(() =>
    recordAvailableCommandsUpdate(OWNER, ALPHA, {
      seq: 2,
      timestamp: "2026-07-23T08:01:00Z",
      payload: { commands: [] },
    }),
  );
  assert.equal(result.current.isOpen, false);
});

test("code context and composing keys leave the literal command alone", async () => {
  const { act } = await import("@testing-library/react");
  const { result } = await setup();
  act(() => result.current.updateQuery("/rev", 4, true));
  assert.equal(result.current.isOpen, false);
  act(() => result.current.updateQuery("/rev", 4));
  assert.equal(
    result.current.handleKeyDown(
      keyEvent("Enter", { nativeEvent: { isComposing: true } }),
    ).handled,
    false,
  );
  assert.equal(
    result.current.handleKeyDown(keyEvent("Tab", { shiftKey: true })).handled,
    false,
  );
});

test("reusing the composer in another channel clears its command query", async () => {
  const { act } = await import("@testing-library/react");
  const { result, rerender } = await setup();
  act(() => result.current.updateQuery("/rev", 4));
  assert.equal(result.current.isOpen, true);
  rerender({ channelId: "other-channel" });
  assert.equal(result.current.isOpen, false);
});
