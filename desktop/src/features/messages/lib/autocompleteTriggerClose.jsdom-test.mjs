/**
 * Clearing a composer draft must close its `#channel` or `:emoji`
 * autocomplete at once. Closing used to wait out the 120 ms query debounce, so
 * an Enter pressed right after clearing picked the stale suggestion instead of
 * submitting (empty-edit-delete.spec.ts covers the `@mention` hook end to end).
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import * as React from "react";

import { ChannelNavigationProvider } from "../../../shared/context/ChannelNavigationContext.tsx";
import { useChannelLinks } from "./useChannelLinks.ts";
import { useEmojiAutocomplete } from "./useEmojiAutocomplete.ts";

afterEach(cleanup);

const channels = [
  {
    id: "general-id",
    name: "general",
    channelType: "stream",
    archivedAt: null,
  },
];
const customEmoji = [
  { shortcode: "partyparrot", url: "https://example.test/partyparrot.png" },
];

const cases = [
  {
    trigger: "#channel",
    draft: "Ship notes for #general",
    renderAutocomplete: () =>
      renderHook(() => useChannelLinks(), {
        wrapper: ({ children }) =>
          React.createElement(
            ChannelNavigationProvider,
            { channels },
            children,
          ),
      }),
    isOpen: (hook) => hook.isChannelOpen,
    update: (hook, value, cursor) => hook.updateChannelQuery(value, cursor),
    keyDown: (hook, event) => hook.handleChannelKeyDown(event),
  },
  {
    trigger: ":emoji",
    draft: "Ship notes :party",
    renderAutocomplete: () =>
      renderHook(() => useEmojiAutocomplete(customEmoji)),
    isOpen: (hook) => hook.isEmojiAutocompleteOpen,
    update: (hook, value, cursor) => hook.updateEmojiQuery(value, cursor),
    keyDown: (hook, event) => hook.handleEmojiKeyDown(event),
  },
];

for (const {
  trigger,
  draft,
  renderAutocomplete,
  isOpen,
  update,
  keyDown,
} of cases) {
  test(`clearing a ${trigger} draft closes its autocomplete before Enter`, async () => {
    const { result } = renderAutocomplete();

    act(() => update(result.current, draft, draft.length));
    // Typing a query still debounces.
    assert.equal(isOpen(result.current), false);
    await waitFor(() => assert.equal(isOpen(result.current), true));

    act(() => update(result.current, "", 0));
    assert.equal(isOpen(result.current), false);
    let prevented = false;
    const enter = {
      key: "Enter",
      altKey: false,
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
      preventDefault: () => {
        prevented = true;
      },
    };
    assert.deepEqual(keyDown(result.current, enter), { handled: false });
    assert.equal(prevented, false);
  });
}
