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

test("always addressing an agent keeps autocomplete open, adds the lock, and pulses", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  let cancelCount = 0;
  const text = "Ask @Agent Ada later @";
  const mentions = {
    cancelMentionAutocomplete: () => {
      cancelCount += 1;
    },
    getDraftMentionRefs: () => [
      {
        displayName: "Agent Ada",
        pubkey: "agent-pubkey",
        isAgent: true,
      },
    ],
    getMentionDisplayName: () => "Agent Ada",
    mentionStartIndex: text.lastIndexOf("@"),
  };
  const audience = {
    pubkeys: [],
    addPubkey: (pubkey) => addedPubkeys.push(pubkey),
  };
  const richText = {
    getPlainTextAndCursor: () => ({ text, cursor: text.length }),
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience,
      audienceScope: "channel-scope",
      mentions,
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText,
    }),
  );

  act(() => {
    result.current.toggleAlwaysAddressAgent({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(appliedEdits, []);
  assert.equal(cancelCount, 0);
  assert.deepEqual(addedPubkeys, ["agent-pubkey"]);
  assert.deepEqual(pulsedPubkeys, ["agent-pubkey"]);
  assert.equal(
    result.current.announcement,
    "Automatically mentioning Agent Ada",
  );
});

test("toggling an addressed agent keeps autocomplete open and removes the lock", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const removedPubkeys = [];
  const pulsedPubkeys = [];
  let cancelCount = 0;
  const text = "Ask @Agent Ada later @";
  const mentions = {
    cancelMentionAutocomplete: () => {
      cancelCount += 1;
    },
    getDraftMentionRefs: () => [
      {
        displayName: "Agent Ada",
        pubkey: "agent-pubkey",
        isAgent: true,
      },
    ],
    getMentionDisplayName: () => "Agent Ada",
    mentionStartIndex: text.lastIndexOf("@"),
  };
  const audience = {
    pubkeys: ["agent-pubkey"],
    addPubkey: () => {
      throw new Error("an addressed agent must not be added again");
    },
    removePubkey: (pubkey) => removedPubkeys.push(pubkey),
  };
  const richText = {
    getPlainTextAndCursor: () => ({ text, cursor: text.length }),
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience,
      audienceScope: "channel-scope",
      mentions,
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText,
    }),
  );

  act(() => {
    result.current.toggleAlwaysAddressAgent({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(appliedEdits, []);
  assert.equal(cancelCount, 0);
  assert.deepEqual(removedPubkeys, ["agent-pubkey"]);
  assert.deepEqual(pulsedPubkeys, []);
  assert.equal(
    result.current.announcement,
    "Stopped automatically mentioning Agent Ada",
  );
});

test("selecting an already addressed agent from the explicit picker pulses its badge", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: () => [],
    getMentionDisplayName: () => "Agent Ada",
    isInlineMentionSelection: () => false,
    insertMention: () => {
      throw new Error("an already addressed agent must not be inserted");
    },
    mentionStartIndex: 5,
  };
  const audience = {
    pubkeys: ["agent-pubkey"],
    addPubkey: (pubkey) => addedPubkeys.push(pubkey),
  };
  const richText = {
    getPlainTextAndCursor: () => ({ text: "ping ", cursor: 5 }),
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience,
      audienceScope: "channel-scope",
      mentions,
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText,
    }),
  );

  act(() => {
    result.current.selectMentionSuggestion({
      pubkey: "AGENT-PUBKEY",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(appliedEdits, []);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, ["agent-pubkey"]);
});

test("selecting an agent from a typed query leaves the inline mention for send", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: () => [],
    getMentionDisplayName: () => "Agent Ada",
    isInlineMentionSelection: () => true,
    insertMention: () => ({
      replaceFromOffset: 5,
      replaceToOffset: 6,
      insertText: "@Agent Ada ",
    }),
    mentionStartIndex: 5,
  };
  const audience = {
    pubkeys: [],
    addPubkey: (pubkey) => addedPubkeys.push(pubkey),
  };
  const richText = {
    // Selection intent comes from the mention picker, even if focus movement
    // makes the editor text/cursor insufficient to re-detect the typed query.
    getPlainTextAndCursor: () => ({ text: "ping ", cursor: 5 }),
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience,
      audienceScope: "channel-scope",
      mentions,
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText,
    }),
  );

  act(() => {
    result.current.selectMentionSuggestion({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 5,
      replaceToOffset: 6,
      insertText: "@Agent Ada ",
    },
  ]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, []);
  assert.equal(result.current.announcement, "");
});

test("selecting an agent from the explicit picker auto-addresses it", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: () => [],
    getMentionDisplayName: () => "Agent Ada",
    isInlineMentionSelection: () => false,
    insertMention: () => {
      throw new Error("explicit picker selections must become addressing");
    },
    mentionStartIndex: 5,
  };
  const audience = {
    pubkeys: [],
    addPubkey: (pubkey) => addedPubkeys.push(pubkey),
  };
  const richText = {
    getPlainTextAndCursor: () => ({ text: "ping ", cursor: 5 }),
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience,
      audienceScope: "channel-scope",
      mentions,
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText,
    }),
  );

  act(() => {
    result.current.selectMentionSuggestion({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(appliedEdits, []);
  assert.deepEqual(addedPubkeys, ["agent-pubkey"]);
  assert.deepEqual(pulsedPubkeys, ["agent-pubkey"]);
  assert.equal(
    result.current.announcement,
    "Automatically mentioning Agent Ada",
  );
});

test("selecting an explicitly unpinned agent inserts a mention until send", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const removedPubkeys = [];
  const pulsedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: () => [],
    getMentionDisplayName: () => "Agent Ada",
    isInlineMentionSelection: () => false,
    insertMention: () => ({
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@Agent Ada ",
    }),
    mentionStartIndex: 0,
  };
  const richText = {
    getPlainTextAndCursor: () => ({ text: "", cursor: 0 }),
  };
  const { result, rerender } = renderHook(
    ({ pubkeys }) =>
      useAgentAddressLockPicker({
        applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
        audience: {
          pubkeys,
          addPubkey: (pubkey) => addedPubkeys.push(pubkey),
          removePubkey: (pubkey) => removedPubkeys.push(pubkey),
        },
        audienceScope: "channel-scope",
        mentions,
        onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
        richText,
      }),
    { initialProps: { pubkeys: ["agent-pubkey"] } },
  );

  act(() => result.current.removeAddressedAgent("AGENT-PUBKEY"));
  rerender({ pubkeys: [] });
  act(() => {
    result.current.selectMentionSuggestion({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });

  assert.deepEqual(removedPubkeys, ["agent-pubkey"]);
  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@Agent Ada ",
    },
  ]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, []);
});

test("an addressed agent keeps its resolved name while mention state clears during send", async () => {
  const { renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  let displayName = "Agent Ada";
  const mentions = {
    getMentionDisplayName: () => displayName,
  };
  const audience = {
    pubkeys: ["agent-pubkey"],
  };
  const { result, rerender } = renderHook(
    ({ profiles }) =>
      useAgentAddressLockPicker({
        applyAutocompleteEdit: () => {},
        audience,
        audienceScope: "channel-scope",
        mentions,
        onPulseAddressLock: () => {},
        profiles,
        richText: {},
      }),
    { initialProps: { profiles: {} } },
  );

  assert.equal(result.current.lockedAgents[0].displayName, "Agent Ada");

  displayName = null;
  rerender({ profiles: {} });

  assert.equal(result.current.lockedAgents[0].displayName, "Agent Ada");
});
