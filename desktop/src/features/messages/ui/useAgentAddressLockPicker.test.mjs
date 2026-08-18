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

test("always mentioning an agent removes the active query, adds the lock, and closes the picker", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getMentionDisplayName: () => "Agent Ada",
    mentionStartIndex: 5,
  };
  const audience = {
    pubkeys: [],
    addPubkey: (pubkey) => addedPubkeys.push(pubkey),
  };
  const richText = {
    getPlainTextAndCursor: () => ({ text: "ping @", cursor: 6 }),
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience,
      audienceScope: "channel-scope",
      mentions,
      richText,
    }),
  );

  act(() => {
    result.current.alwaysMentionAgent({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(appliedEdits, [
    { replaceFromOffset: 5, replaceToOffset: 6, insertText: "" },
  ]);
  assert.deepEqual(addedPubkeys, ["agent-pubkey"]);
});
