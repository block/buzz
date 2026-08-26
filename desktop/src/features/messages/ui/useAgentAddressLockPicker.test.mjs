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

test("always addressing an agent keeps autocomplete open, inserts the chip, adds the lock, and pulses", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  let cancelCount = 0;
  const text = "@";
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
    isInlineMentionSelection: () => false,
    isMentionOpen: true,
    registerMentionPubkey: () => {},
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

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@Agent Ada ",
      preserveSelection: true,
    },
  ]);
  assert.equal(cancelCount, 0);
  assert.deepEqual(addedPubkeys, ["agent-pubkey"]);
  assert.deepEqual(pulsedPubkeys, ["agent-pubkey"]);
  assert.equal(
    result.current.announcement,
    "Automatically mentioning Agent Ada",
  );
});

test("always addressing a new agent delegates the first add for immediate confirmation", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const addressedSuggestions = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: () => {},
      audience: {
        pubkeys: [],
        addPubkey: (pubkey) => addedPubkeys.push(pubkey),
      },
      audienceScope: "channel-scope",
      mentions: {
        getDraftMentionRefs: () => [],
        getMentionDisplayName: () => "Agent Ada",
        isInlineMentionSelection: () => false,
        isMentionOpen: false,
        registerMentionPubkey: () => {},
      },
      onAddressAgentMention: (value) => addressedSuggestions.push(value),
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText: {
        getPlainTextAndCursor: () => ({ text: "@Agent Ada ", cursor: 11 }),
      },
    }),
  );

  act(() => result.current.toggleAlwaysAddressAgent(suggestion));

  assert.deepEqual(addressedSuggestions, [suggestion]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, []);
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
    registerMentionPubkey: () => {},
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

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 4,
      replaceToOffset: 15,
      insertText: "",
    },
  ]);
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
    registerMentionPubkey: () => {},
    isInlineMentionSelection: () => false,
    insertMention: () => ({
      replaceFromOffset: 5,
      replaceToOffset: 5,
      insertText: "@Agent Ada ",
    }),
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

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 5,
      replaceToOffset: 5,
      insertText: "@Agent Ada ",
    },
  ]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, ["agent-pubkey"]);
});

test("selecting an agent from a typed query immediately auto-addresses it", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const autoPinnedSuggestions = [];
  const appliedEdits = [];
  const addedPubkeys = [];
  const pulsedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: () => [],
    getMentionDisplayName: () => "Agent Ada",
    registerMentionPubkey: () => {},
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
      onAutoPinAgentMention: (suggestion) =>
        autoPinnedSuggestions.push(suggestion),
      onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
      richText,
    }),
  );

  const suggestion = {
    pubkey: "agent-pubkey",
    displayName: "Agent Ada",
    isAgent: true,
  };
  act(() => result.current.selectMentionSuggestion(suggestion));

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 5,
      replaceToOffset: 6,
      insertText: "@Agent Ada ",
    },
  ]);
  assert.deepEqual(autoPinnedSuggestions, [suggestion]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, []);
  assert.equal(result.current.announcement, "");
});

test("selecting a human mention never changes automatic addressing", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const autoPinnedSuggestions = [];
  const appliedEdits = [];
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience: { pubkeys: [], addPubkey: () => {} },
      audienceScope: "channel-scope",
      mentions: {
        getMentionDisplayName: () => "Alice",
        insertMention: () => ({
          replaceFromOffset: 0,
          replaceToOffset: 3,
          insertText: "@Alice ",
        }),
      },
      onAutoPinAgentMention: (suggestion) =>
        autoPinnedSuggestions.push(suggestion),
      onPulseAddressLock: () => {},
      richText: {
        getPlainTextAndCursor: () => ({ text: "@Al", cursor: 3 }),
      },
    }),
  );

  act(() =>
    result.current.selectMentionSuggestion({
      pubkey: "human-pubkey",
      displayName: "Alice",
      isAgent: false,
    }),
  );

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 0,
      replaceToOffset: 3,
      insertText: "@Alice ",
    },
  ]);
  assert.deepEqual(autoPinnedSuggestions, []);
});

test("removing the last agent chip clears its automatic address", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const removedPubkeys = [];
  const mentionRefsByText = {
    "@Agent Ada first @Agent Ada second": [
      { displayName: "Agent Ada", pubkey: "agent-pubkey", isAgent: true },
      { displayName: "Agent Ada", pubkey: "agent-pubkey", isAgent: true },
    ],
    "@Agent Ada second": [
      { displayName: "Agent Ada", pubkey: "agent-pubkey", isAgent: true },
    ],
    "": [],
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: () => {},
      audience: {
        pubkeys: ["agent-pubkey", "existing-lock"],
        removePubkey: (pubkey) => removedPubkeys.push(pubkey),
      },
      audienceScope: "channel-scope",
      mentions: {
        getDraftMentionRefs: (text) => mentionRefsByText[text] ?? [],
        getMentionDisplayName: () => "Agent Ada",
      },
      onPulseAddressLock: () => {},
      richText: { getPlainTextAndCursor: () => ({ text: "", cursor: 0 }) },
    }),
  );

  act(() => result.current.trackMentionAddressedAgent("agent-pubkey"));
  act(() =>
    result.current.syncAddressedAgentsFromText(
      "@Agent Ada first @Agent Ada second",
    ),
  );
  act(() => result.current.syncAddressedAgentsFromText("@Agent Ada second"));
  assert.deepEqual(removedPubkeys, []);

  act(() => result.current.syncAddressedAgentsFromText(""));
  assert.deepEqual(removedPubkeys, ["agent-pubkey"]);
});

test("removing human mentions is ignored while removing a restored agent chip clears its lock", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const removedPubkeys = [];
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: () => {},
      audience: {
        pubkeys: ["existing-lock"],
        removePubkey: (pubkey) => removedPubkeys.push(pubkey),
      },
      audienceScope: "channel-scope",
      mentions: {
        getDraftMentionRefs: (text) => {
          if (text === "@Alice @Existing Agent") {
            return [
              { displayName: "Alice", pubkey: "human-pubkey", isAgent: false },
              {
                displayName: "Existing Agent",
                pubkey: "existing-lock",
                isAgent: true,
              },
            ];
          }
          return text
            ? [{ displayName: "Alice", pubkey: "human-pubkey", isAgent: false }]
            : [];
        },
        getMentionDisplayName: () => "Existing Agent",
      },
      onPulseAddressLock: () => {},
      richText: { getPlainTextAndCursor: () => ({ text: "", cursor: 0 }) },
    }),
  );

  act(() =>
    result.current.syncAddressedAgentsFromText("@Alice @Existing Agent"),
  );
  act(() => result.current.syncAddressedAgentsFromText("@Alice"));
  assert.deepEqual(removedPubkeys, ["existing-lock"]);
  act(() => result.current.syncAddressedAgentsFromText(""));
  assert.deepEqual(removedPubkeys, ["existing-lock"]);
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
    registerMentionPubkey: () => {},
    isInlineMentionSelection: () => false,
    insertMention: () => ({
      replaceFromOffset: 5,
      replaceToOffset: 5,
      insertText: "@Agent Ada ",
    }),
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

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 5,
      replaceToOffset: 5,
      insertText: "@Agent Ada ",
    },
  ]);
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
    getDraftMentionRefs: () => [
      { displayName: "Agent Ada", pubkey: "agent-pubkey", isAgent: true },
    ],
    getMentionDisplayName: () => "Agent Ada",
    registerMentionPubkey: () => {},
    isInlineMentionSelection: () => false,
    insertMention: () => ({
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@Agent Ada ",
    }),
    mentionStartIndex: 0,
  };
  const richText = {
    getPlainTextAndCursor: () => ({
      text: "@Agent Ada keep this authored text",
      cursor: 35,
    }),
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
  assert.deepEqual(appliedEdits, []);
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
