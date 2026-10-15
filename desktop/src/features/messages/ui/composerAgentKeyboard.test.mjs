import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

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
});

after(() => dom.window.close());

test("agent picker preference skips people", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useMentionSelection } = await import(
    "@/features/messages/lib/useMentionSelection"
  );
  const view = renderHook(
    ({ suggestions }) => useMentionSelection(suggestions),
    { initialProps: { suggestions: [] } },
  );
  const suggestions = [
    { displayName: "Alice", pubkey: "person" },
    { displayName: "Agent Ada", isAgent: true, pubkey: "agent-a" },
    { displayName: "Bob", pubkey: "person-b" },
    { displayName: "Agent Bea", isAgent: true, pubkey: "agent-b" },
  ];

  act(() => view.result.current.prepareSelectionPreference("first-agent"));
  view.rerender({ suggestions });
  assert.equal(view.result.current.mentionSelectedIndex, 1);
});

test("primary+Shift+M addresses the default agent or toggles the tray selection in place", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAlwaysAddressShortcut } = await import(
    "./useAlwaysAddressShortcut.ts"
  );
  const { isMacPlatform } = await import("@/shared/lib/platform");
  const toggled = [];
  const suggestion = {
    displayName: "Agent Ada",
    isAgent: true,
    pubkey: "agent-a",
  };
  const createEvent = () => ({
    altKey: false,
    code: "KeyM",
    ctrlKey: !isMacPlatform(),
    key: "M",
    metaKey: isMacPlatform(),
    preventDefault() {},
    repeat: false,
    shiftKey: true,
  });
  const view = renderHook(
    ({ isMentionOpen }) =>
      useAlwaysAddressShortcut({
        enabled: true,
        mentions: {
          getDefaultAgentSuggestion: () => suggestion,
          isMentionOpen,
          mentionSelectedIndex: 0,
          suggestions: [suggestion],
        },
        onOpenPicker: () => {},
        onToggle: (value) => toggled.push(value),
      }),
    { initialProps: { isMentionOpen: false } },
  );

  act(() => assert.equal(view.result.current(createEvent()), true));
  assert.deepEqual(toggled, [suggestion]);

  view.rerender({ isMentionOpen: true });
  act(() => assert.equal(view.result.current(createEvent()), true));
  assert.deepEqual(toggled, [suggestion, suggestion]);

  act(() => assert.equal(view.result.current(createEvent()), true));
  assert.deepEqual(toggled, [suggestion, suggestion, suggestion]);
});

test("primary+Shift+M removes the current locked agent before choosing a new default", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAlwaysAddressShortcut } = await import(
    "./useAlwaysAddressShortcut.ts"
  );
  const { isMacPlatform } = await import("@/shared/lib/platform");
  const lockedAgent = {
    avatarUrl: null,
    displayName: "Agent Ada",
    pubkey: "agent-a",
  };
  const defaultAgent = {
    displayName: "Agent Bea",
    isAgent: true,
    pubkey: "agent-b",
  };
  const toggled = [];
  const { result } = renderHook(() =>
    useAlwaysAddressShortcut({
      enabled: true,
      lockedAgent,
      mentions: {
        getDefaultAgentSuggestion: () => defaultAgent,
        isMentionOpen: false,
        mentionSelectedIndex: 0,
        suggestions: [],
      },
      onOpenPicker: () => {},
      onToggle: (value) => toggled.push(value),
    }),
  );

  act(() =>
    assert.equal(
      result.current({
        altKey: false,
        code: "KeyM",
        ctrlKey: !isMacPlatform(),
        key: "M",
        metaKey: isMacPlatform(),
        preventDefault() {},
        repeat: false,
        shiftKey: true,
      }),
      true,
    ),
  );

  assert.deepEqual(toggled, [{ ...lockedAgent, isAgent: true }]);
});

test("primary+Shift+M opens the picker when no default agent is ready", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAlwaysAddressShortcut } = await import(
    "./useAlwaysAddressShortcut.ts"
  );
  const { isMacPlatform } = await import("@/shared/lib/platform");
  let opened = 0;
  const { result } = renderHook(() =>
    useAlwaysAddressShortcut({
      enabled: true,
      mentions: {
        getDefaultAgentSuggestion: () => null,
        isMentionOpen: false,
        mentionSelectedIndex: 0,
        suggestions: [],
      },
      onOpenPicker: () => {
        opened += 1;
      },
      onToggle: () => {},
    }),
  );

  act(() =>
    assert.equal(
      result.current({
        altKey: false,
        code: "KeyM",
        ctrlKey: !isMacPlatform(),
        key: "M",
        metaKey: isMacPlatform(),
        preventDefault() {},
        repeat: false,
        shiftKey: true,
      }),
      true,
    ),
  );

  assert.equal(opened, 1);
});
