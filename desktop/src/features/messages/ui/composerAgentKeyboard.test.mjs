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

test("primary+Shift+Enter opens the picker or toggles in place", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAlwaysAddressShortcut } = await import(
    "./useAlwaysAddressShortcut.ts"
  );
  const { isMacPlatform } = await import("@/shared/lib/platform");
  const opened = [];
  const toggled = [];
  const suggestion = {
    displayName: "Agent Ada",
    isAgent: true,
    pubkey: "agent-a",
  };
  const createEvent = () => ({
    altKey: false,
    ctrlKey: !isMacPlatform(),
    key: "Enter",
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
          isMentionOpen,
          mentionSelectedIndex: 0,
          suggestions: [suggestion],
        },
        onOpenPicker: (insertTrigger) => opened.push(insertTrigger),
        onToggle: (value) => toggled.push(value),
      }),
    { initialProps: { isMentionOpen: false } },
  );

  act(() => assert.equal(view.result.current(createEvent()), true));
  assert.deepEqual(opened, [false]);
  assert.deepEqual(toggled, []);

  view.rerender({ isMentionOpen: true });
  act(() => assert.equal(view.result.current(createEvent()), true));
  assert.deepEqual(toggled, [suggestion]);

  act(() => assert.equal(view.result.current(createEvent()), true));
  assert.deepEqual(toggled, [suggestion, suggestion]);
});
