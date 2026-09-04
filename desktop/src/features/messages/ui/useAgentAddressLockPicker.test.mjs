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
  const openPickerCalls = [];
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
    openMentionPicker: (...args) => openPickerCalls.push(args),
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
      reassertMentionCaret: false,
    },
  ]);
  assert.equal(cancelCount, 0);
  assert.deepEqual(openPickerCalls, [[text.length, "preserve"]]);
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

test("unpinning an addressed agent keeps its current mention and autocomplete open", async () => {
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
    result.current.toggleAlwaysAddressAgent(
      {
        pubkey: "agent-pubkey",
        displayName: "Agent Ada",
        isAgent: true,
      },
      { preserveMention: true },
    );
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
      onAutoPinAgentMention: (suggestion, options) =>
        autoPinnedSuggestions.push([suggestion, options]),
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
  assert.deepEqual(autoPinnedSuggestions, [
    [suggestion, { reinstateExcluded: true }],
  ]);
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

test("restoring a multi-word automatic mention into an empty composer focuses after its trailing space", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const registeredMentions = [];
  let focusEndCount = 0;
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience: {
        pubkeys: ["agent-pubkey"],
        addPubkey: () => {},
      },
      audienceScope: "thread-scope",
      mentions: {
        getDraftMentionRefs: () => [],
        getMentionDisplayName: () => "claude code",
        registerMentionPubkey: (...args) => registeredMentions.push(args),
      },
      onPulseAddressLock: () => {},
      richText: {
        focusEnd: () => {
          focusEndCount += 1;
        },
        getPlainTextAndCursor: () => ({ text: "", cursor: 0 }),
      },
    }),
  );

  act(() => result.current.restoreAddressedAgentMentions());

  assert.deepEqual(registeredMentions, [
    ["claude code", "agent-pubkey", { isAgent: true }],
  ]);
  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@claude code ",
      preserveSelection: true,
    },
  ]);
  assert.equal(focusEndCount, 1);
});

test("restoring before authored text preserves its selection", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  let focusEndCount = 0;
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience: {
        pubkeys: ["agent-pubkey"],
        addPubkey: () => {},
      },
      audienceScope: "thread-scope",
      mentions: {
        getDraftMentionRefs: () => [],
        getMentionDisplayName: () => "Morgarita",
        registerMentionPubkey: () => {},
      },
      onPulseAddressLock: () => {},
      richText: {
        focusEnd: () => {
          focusEndCount += 1;
        },
        getPlainTextAndCursor: () => ({ text: "draft text", cursor: 10 }),
      },
    }),
  );

  act(() => result.current.restoreAddressedAgentMentions());

  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@Morgarita ",
      preserveSelection: true,
    },
  ]);
  assert.equal(focusEndCount, 0);
});

test("restoring an existing automatic mention re-registers its agent chip", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const registeredMentions = [];
  const syncedAddressedNames = [];
  let focusEndCount = 0;
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
      audience: {
        pubkeys: ["agent-pubkey"],
        addPubkey: () => {},
      },
      audienceScope: "thread-scope",
      mentions: {
        getDraftMentionRefs: () => [],
        getMentionDisplayName: () => "claude code",
        registerMentionPubkey: (...args) => registeredMentions.push(args),
      },
      onPulseAddressLock: () => {},
      richText: {
        focusEnd: () => {
          focusEndCount += 1;
        },
        getPlainTextAndCursor: () => ({
          text: "@claude code ",
          cursor: 13,
        }),
        syncAddressedAgentMentionNames: (names) =>
          syncedAddressedNames.push(names),
      },
    }),
  );

  act(() => result.current.restoreAddressedAgentMentions());

  assert.deepEqual(registeredMentions, [
    ["claude code", "agent-pubkey", { isAgent: true }],
  ]);
  assert.deepEqual(appliedEdits, []);
  assert.equal(focusEndCount, 0);
  assert.deepEqual(syncedAddressedNames.at(-1), ["claude code"]);
});

test("deleting the last automatic agent mention explicitly excludes its address", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const excludedPubkeys = [];
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
        excludePubkey: (pubkey) => excludedPubkeys.push(pubkey),
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
  assert.deepEqual(excludedPubkeys, ["agent-pubkey"]);
  assert.deepEqual(removedPubkeys, []);
});

test("deleting human mentions is ignored while deleting a restored automatic agent mention excludes its address", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const excludedPubkeys = [];
  const removedPubkeys = [];
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      applyAutocompleteEdit: () => {},
      audience: {
        pubkeys: ["existing-lock"],
        excludePubkey: (pubkey) => excludedPubkeys.push(pubkey),
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
  assert.deepEqual(excludedPubkeys, ["existing-lock"]);
  assert.deepEqual(removedPubkeys, []);
  act(() => result.current.syncAddressedAgentsFromText(""));
  assert.deepEqual(excludedPubkeys, ["existing-lock"]);
  assert.deepEqual(removedPubkeys, []);
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

test("repeatedly selecting an explicitly unpinned agent keeps its mentions manual", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const addedPubkeys = [];
  const autoPinnedSuggestions = [];
  const removedPubkeys = [];
  const pulsedPubkeys = [];
  const mentions = {
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: () => [
      { displayName: "Agent Ada", pubkey: "agent-pubkey", isAgent: true },
    ],
    getMentionDisplayName: () => "Agent Ada",
    registerMentionPubkey: () => {},
    isInlineMentionSelection: () => true,
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
        onAutoPinAgentMention: (suggestion, options) =>
          autoPinnedSuggestions.push([suggestion, options]),
        onPulseAddressLock: (pubkey) => pulsedPubkeys.push(pubkey),
        richText,
      }),
    { initialProps: { pubkeys: ["agent-pubkey"] } },
  );

  act(() => result.current.removeAddressedAgent("AGENT-PUBKEY"));
  assert.deepEqual(appliedEdits, [
    {
      replaceFromOffset: 0,
      replaceToOffset: 11,
      insertText: "",
    },
  ]);
  appliedEdits.length = 0;
  rerender({ pubkeys: [] });
  act(() => {
    result.current.selectMentionSuggestion({
      pubkey: "agent-pubkey",
      displayName: "Agent Ada",
      isAgent: true,
    });
  });
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
    {
      replaceFromOffset: 0,
      replaceToOffset: 0,
      insertText: "@Agent Ada ",
    },
  ]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(autoPinnedSuggestions, [
    [
      {
        pubkey: "agent-pubkey",
        displayName: "Agent Ada",
        isAgent: true,
      },
      { reinstateExcluded: false },
    ],
    [
      {
        pubkey: "agent-pubkey",
        displayName: "Agent Ada",
        isAgent: true,
      },
      { reinstateExcluded: false },
    ],
  ]);
  assert.deepEqual(pulsedPubkeys, []);
});

test("restoring after an agent rename keeps the existing automatic mention", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const appliedEdits = [];
  const registeredMentions = [];
  const oldName = "OldName";
  const newName = "NewName";
  const { result, rerender } = renderHook(
    ({ displayName }) =>
      useAgentAddressLockPicker({
        applyAutocompleteEdit: (edit) => appliedEdits.push(edit),
        audience: { pubkeys: ["agent-pubkey"], addPubkey: () => {} },
        audienceScope: "channel-scope",
        mentions: {
          getDraftMentionRefs: () => [
            { displayName: oldName, pubkey: "agent-pubkey", isAgent: true },
          ],
          getMentionDisplayName: () => displayName,
          registerMentionPubkey: (...args) => registeredMentions.push(args),
        },
        onPulseAddressLock: () => {},
        profiles: {},
        richText: {
          getPlainTextAndCursor: () => ({
            text: `@${oldName} authored draft`,
            cursor: 23,
          }),
        },
      }),
    { initialProps: { displayName: oldName } },
  );

  rerender({ displayName: newName });
  act(() => result.current.restoreAddressedAgentMentions());

  assert.deepEqual(appliedEdits, []);
  assert.deepEqual(registeredMentions, [
    [newName, "agent-pubkey", { isAgent: true }],
  ]);
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
