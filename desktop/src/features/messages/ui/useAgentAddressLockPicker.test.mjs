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
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
    cancelMentionAutocomplete: () => {
      cancelCount += 1;
    },
    getDraftMentionRefs: () => [],
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
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
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
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
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
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
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
        getDraftMentionRefs: () => [],
        getMentionDisplayName: () => "claude code",
        registerMentionPubkey: (...args) => {
          registeredMentions.push(args);
          return args[0];
        },
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
        getDraftMentionRefs: () =>
          registeredMentions.length
            ? [
                {
                  displayName: "claude code",
                  pubkey: "agent-pubkey",
                  isAgent: true,
                },
              ]
            : [],
        getMentionDisplayName: () => "claude code",
        registerMentionPubkey: (...args) => {
          registeredMentions.push(args);
          return args[0];
        },
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
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
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
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
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
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
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
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
          retireMentionSelection: () => {},
          admitMentionSelection: (_suggestion, _read, commit) => commit(),
          cancelMentionAutocomplete: () => {},
          getDraftMentionRefs: () => [
            { displayName: oldName, pubkey: "agent-pubkey", isAgent: true },
          ],
          getMentionDisplayName: () => displayName,
          registerMentionPubkey: (...args) => {
            registeredMentions.push(args);
            return args[0];
          },
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
    [oldName, "agent-pubkey", { isAgent: true }],
  ]);
});

test("an addressed agent keeps its resolved name while mention state clears during send", async () => {
  const { renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  let displayName = "Agent Ada";
  const mentions = {
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
    cancelMentionAutocomplete: () => {},
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

test("automatic mention insertion and restoration use the registered collision-safe label", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const { selectedMentionLabel, extractMentionPubkeys } = await import(
    "../lib/extractMentionPubkeys.ts"
  );
  const { snapshotDraftMentionRefs } = await import(
    "../lib/draftMentionRefs.ts"
  );
  const local = "a".repeat(64);
  const remote = "e".repeat(64);
  const bindings = new Map([["carl", local]]);
  let text = "@carl existing";
  const prefixes = [];
  const mentions = {
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: (value) =>
      snapshotDraftMentionRefs(value, bindings, [...bindings.keys()]),
    getMentionDisplayName: (pubkey) =>
      [...bindings].find(([, key]) => key === pubkey)?.[0] ?? "carl",
    registerMentionPubkey: (name, pubkey) => {
      const label = selectedMentionLabel(name, pubkey, bindings);
      bindings.set(label, pubkey);
      return label;
    },
    isMentionOpen: false,
  };
  const { result, rerender } = renderHook(
    ({ pubkeys }) =>
      useAgentAddressLockPicker({
        audience: { pubkeys, addPubkey: () => {}, removePubkey: () => {} },
        audienceScope: "channel",
        mentions,
        profiles: { [remote]: { displayName: "carl" } },
        onPulseAddressLock: () => {},
        onImplicitPrefixInserted: (value) => prefixes.push(...value),
        applyAutocompleteEdit: (edit) => {
          text =
            text.slice(0, edit.replaceFromOffset) +
            edit.insertText +
            text.slice(edit.replaceToOffset);
        },
        richText: {
          getPlainTextAndCursor: () => ({ text, cursor: text.length }),
          focusEnd: () => {},
        },
      }),
    { initialProps: { pubkeys: [] } },
  );
  act(() =>
    result.current.toggleAlwaysAddressAgent({
      displayName: "carl",
      pubkey: remote,
      isAgent: true,
    }),
  );
  assert.equal(text, `@carl (${remote}) @carl existing`);
  assert.deepEqual(
    extractMentionPubkeys({
      text,
      selectedMentions: bindings,
      memberCandidates: [],
    }),
    [local, remote],
  );
  rerender({ pubkeys: [remote] });
  act(() => result.current.restoreAddressedAgentMentions());
  assert.equal(
    text,
    `@carl (${remote}) @carl existing`,
    "restore must not append or rebind the local mention",
  );
  act(() => result.current.removeAddressedAgent(remote));
  assert.equal(
    text,
    "@carl existing",
    "unpin removes the qualified prefix only",
  );
  text = "";
  act(() => result.current.restoreAddressedAgentMentions([remote], [remote]));
  assert.equal(text, `@carl (${remote}) `);
  assert.equal(prefixes.at(-1).prefix, `@carl (${remote}) `);
  assert.deepEqual(
    extractMentionPubkeys({
      text,
      selectedMentions: bindings,
      memberCandidates: [],
    }),
    [remote],
  );
});

test("inverse deletion and toggle preserve B and exclude A from the composed send recipients", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const { selectedMentionLabel, extractMentionPubkeys } = await import(
    "../lib/extractMentionPubkeys.ts"
  );
  const { snapshotDraftMentionRefs } = await import(
    "../lib/draftMentionRefs.ts"
  );
  const { mergeMentionRecipients } = await import(
    "./useMentionSendFlow.helpers.ts"
  );
  const A = "a".repeat(64),
    B = "b".repeat(64);
  const bindings = new Map([["Scout", A]]);
  const qualified = selectedMentionLabel("Scout", B, bindings);
  bindings.set(qualified, B);
  let text = `@Scout @${qualified} hello`;
  const excluded = [],
    edits = [];
  const mentions = {
    retireMentionSelection: () => {},
    admitMentionSelection: (_suggestion, _read, commit) => commit(),
    cancelMentionAutocomplete: () => {},
    getDraftMentionRefs: (value) =>
      snapshotDraftMentionRefs(value, bindings, [...bindings.keys()]),
    getMentionDisplayName: (key) =>
      [...bindings].find(([, k]) => k === key)?.[0],
    registerMentionPubkey: (name, key) => {
      const label = selectedMentionLabel(name, key, bindings);
      bindings.set(label, key);
      return label;
    },
    isMentionOpen: false,
  };
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      audience: { pubkeys: [A, B], excludePubkey: (key) => excluded.push(key) },
      audienceScope: "channel",
      mentions,
      onPulseAddressLock: () => {},
      applyAutocompleteEdit: (edit) => {
        edits.push(edit);
        text =
          text.slice(0, edit.replaceFromOffset) +
          edit.insertText +
          text.slice(edit.replaceToOffset);
      },
      richText: {
        getPlainTextAndCursor: () => ({ text, cursor: text.length }),
      },
    }),
  );
  act(() => {
    result.current.trackMentionAddressedAgent(A);
    result.current.trackMentionAddressedAgent(B);
  });
  // The user deletes only the first (unqualified A) mention.
  text = `@${qualified} hello`;
  act(() => result.current.syncAddressedAgentsFromText(text));
  const explicit = extractMentionPubkeys({
    text,
    selectedMentions: bindings,
    memberCandidates: [],
  });
  const merged = mergeMentionRecipients(
    explicit,
    [A, B].filter((k) => !excluded.includes(k)),
  );
  assert.deepEqual(explicit, [B]);
  assert.deepEqual(excluded, [A]);
  assert.deepEqual(merged, [B]);

  // Toggling off A should not touch B's qualified mention.
  text = `@Scout @${qualified} hello`;
  act(() =>
    result.current.toggleAlwaysAddressAgent({
      displayName: "Scout",
      pubkey: A,
      isAgent: true,
    }),
  );
  assert.equal(text, `@${qualified} hello`);

  const afterToggle = mergeMentionRecipients(
    extractMentionPubkeys({
      text,
      selectedMentions: bindings,
      memberCandidates: [],
    }),
    [A, B].filter((k) => !excluded.includes(k)),
  );
  assert.deepEqual(afterToggle, [B]);
});

test("implicit prefix removal uses the present exact label rather than a stale alias", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const key = "a".repeat(64);
  let text = "@Historical Scout hello";
  const { result } = renderHook(() =>
    useAgentAddressLockPicker({
      audience: { pubkeys: [key], excludePubkey: () => {} },
      audienceScope: "channel",
      mentions: {
        retireMentionSelection: () => {},
        admitMentionSelection: (_suggestion, _read, commit) => commit(),
        cancelMentionAutocomplete: () => {},
        getDraftMentionRefs: () => [
          { displayName: "Historical Scout", pubkey: key, isAgent: true },
        ],
        getMentionDisplayName: () => "Scout",
      },
      onPulseAddressLock: () => {},
      applyAutocompleteEdit: (edit) => {
        text =
          text.slice(0, edit.replaceFromOffset) +
          edit.insertText +
          text.slice(edit.replaceToOffset);
      },
      richText: {
        getPlainTextAndCursor: () => ({ text, cursor: text.length }),
      },
    }),
  );
  act(() => result.current.removeAddressedAgent(key));
  assert.equal(text, "hello");
});

// Integrated selection owner: real admission and authorization, deferred at the
// directory boundary, not a fake boolean permission supplied by the picker.
test("displayed identity survives reranking; fresh revocation and retired callbacks cannot select or pin", async () => {
  const React = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const { useMentionAdmission, useStableMentionSuggestions } = await import(
    "../lib/useMentionAdmission.ts"
  );
  const { revalidateAgentMentionPubkeys } = await import(
    "../lib/agentMentionRevalidation.ts"
  );
  const agent = "b".repeat(64),
    other = "c".repeat(64),
    self = "a".repeat(64);
  const first = {
    kind: "identity",
    pubkey: agent,
    displayName: "Displayed",
    isAgent: true,
  };
  const second = {
    kind: "identity",
    pubkey: other,
    displayName: "Other",
    isAgent: true,
  };
  let incoming = [first, second];
  let allowed = true;
  let release;
  let deferred = false;
  let position = { text: "@", cursor: 1 };
  const inserted = [],
    pinned = [],
    requested = [];
  let generation;
  const { result, rerender } = renderHook(() => {
    generation = React.useRef(0);
    const rows = useStableMentionSuggestions(
      String(generation.current),
      incoming,
    );
    const admit = useMentionAdmission({
      generation,
      isPending: () => false,
      channelId: "general",
      channelType: "channel",
      currentPubkey: self,
      agentPubkeys: new Set([agent, other]),
      personaIds: new Set(),
      revalidate: (keys) =>
        revalidateAgentMentionPubkeys({
          pubkeys: keys,
          agentPubkeys: new Set([agent, other]),
          currentPubkey: self,
          eligibilityScope: { type: "channel", channelId: "general" },
          phase: "prepare",
          sharedChannelIds: new Set(),
          refetchManagedAgents: async () => ({ data: [], error: null }),
          fetchRelayAgents: async (keys) => {
            requested.push(keys);
            if (deferred)
              await new Promise((resolve) => {
                release = resolve;
              });
            return allowed
              ? keys.map((pubkey) => ({
                  pubkey,
                  respondTo: "anyone",
                  channelIds: ["general"],
                }))
              : [];
          },
        }),
    });
    const picker = useAgentAddressLockPicker({
      applyAutocompleteEdit: (edit) => inserted.push(edit),
      audience: { pubkeys: [], addPubkey: (key) => pinned.push(key) },
      audienceScope: "general",
      mentions: {
        retireMentionSelection: () => {
          generation.current++;
        },
        admitMentionSelection: admit,
        cancelMentionAutocomplete: () => {
          generation.current++;
        },
        getMentionDisplayName: () => null,
        getDraftMentionRefs: () => [],
        insertMention: (row) => ({
          insertText: row.displayName,
          pubkey: row.pubkey,
        }),
        registerMentionPubkey: (label) => label,
        isInlineMentionSelection: () => false,
        isMentionOpen: false,
      },
      richText: { getPlainTextAndCursor: () => position },
      onPulseAddressLock: () => {},
    });
    return { rows, ...picker };
  });
  incoming = [second, { ...first, displayName: "Renamed" }];
  rerender();
  assert.deepEqual(
    result.current.rows.map((row) => row.pubkey),
    [agent, other],
  );
  assert.equal(result.current.rows[0].displayName, "Displayed");
  await act(async () => {
    await result.current.selectMentionSuggestion(result.current.rows[0]);
  });
  assert.deepEqual(requested.pop(), [agent]);
  assert.equal(inserted[0].pubkey, agent);
  assert.deepEqual(pinned, [agent]);
  inserted.length = 0;
  pinned.length = 0;
  allowed = false;
  await act(async () => {
    await result.current.selectMentionSuggestion(result.current.rows[0]);
    await result.current.toggleAlwaysAddressAgent(result.current.rows[0]);
  });
  assert.deepEqual(inserted, []);
  assert.deepEqual(pinned, []);
  allowed = true;
  deferred = true;
  let pending;
  const { QueryClient } = await import("@tanstack/react-query");
  const {
    refreshDirectoryAfterMembershipChange,
    resetMembershipDirectorySync,
  } = await import("../../channels/membershipDirectorySync.ts");
  const client = new QueryClient();
  for (const retire of [
    () => refreshDirectoryAfterMembershipChange(client, "accepted-removal"),
    resetMembershipDirectorySync,
  ]) {
    act(() => {
      pending = result.current.selectMentionSuggestion(result.current.rows[0]);
    });
    retire(); // the still-positive direct RPC was started before this write/reset
    release();
    await act(async () => {
      await pending;
    });
    assert.deepEqual(inserted, []);
    assert.deepEqual(pinned, []);
  }
  client.clear();
  act(() => {
    pending = result.current.selectMentionSuggestion(result.current.rows[0]);
  });
  generation.current++; // editor cancellation / A -> B -> A still retires
  release();
  await act(async () => {
    await pending;
  });
  assert.deepEqual(inserted, []);
  assert.deepEqual(pinned, []);
  const stale = result.current.selectMentionSuggestion;
  rerender();
  const before = requested.length;
  await act(async () => {
    await stale(first);
  });
  assert.equal(requested.length, before);
  act(() => {
    pending = result.current.toggleAlwaysAddressAgent(result.current.rows[0]);
  });
  position = { text: "@changed", cursor: 8 };
  document.dispatchEvent(new dom.window.Event("input"));
  position = { text: "@", cursor: 1 };
  release();
  await act(async () => {
    await pending;
  });
  assert.deepEqual(inserted, []);
  assert.deepEqual(pinned, []);
});

test("selection reads fresh human roster/visibility with private-member, open, unknown and DM rules", async () => {
  const React = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { useMentionAdmission } = await import("../lib/useMentionAdmission.ts");
  const human = "c".repeat(64),
    self = "a".repeat(64);
  let visibility = "private",
    members = [],
    channelType = "channel";
  let writes = 0,
    reads = 0;
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (name) => {
      reads++;
      if (name === "get_channel_members") return { members };
      if (name === "get_channels")
        return {
          hash: "fresh",
          channels: [{ id: "general", channel_type: "channel", visibility }],
          last_messages: {},
        };
      throw new Error(`Unexpected ${name}`);
    },
  };
  const { result, rerender } = renderHook(() =>
    useMentionAdmission({
      generation: React.useRef(0),
      isPending: () => false,
      channelId: "general",
      channelType,
      currentPubkey: self,
      agentPubkeys: new Set(),
      personaIds: new Set(),
      revalidate: async (keys) => keys,
    }),
  );
  const select = async () => {
    let accepted;
    await act(async () => {
      accepted = await result.current(
        { pubkey: human, displayName: "Human" },
        () => ({ text: "@hu", cursor: 3 }),
        () => writes++,
      );
    });
    return accepted;
  };
  assert.equal(
    await select(),
    false,
    "revoked private membership denies nonmember admission",
  );
  members = [{ pubkey: self, role: "member" }];
  assert.equal(await select(), true, "ordinary private member can add");
  members = [];
  visibility = "open";
  assert.equal(await select(), true, "open permits a nonmember");
  visibility = undefined;
  assert.equal(await select(), false, "unknown visibility fails closed");
  members = [{ pubkey: human, role: "member" }];
  assert.equal(await select(), true, "existing member needs no add permission");
  channelType = "dm";
  rerender();
  const before = reads;
  assert.equal(
    await select(),
    true,
    "DM mention does not mutate immutable membership",
  );
  assert.equal(reads, before);
  assert.equal(writes, 4);
  delete dom.window.__TAURI_INTERNALS__;
});

test("production editor lifecycle retires caret departures, silent restores and external blur, not options focus", async () => {
  const { renderHook } = await import("@testing-library/react");
  const { useMentionAdmissionEditor } = await import(
    "../lib/useMentionAdmissionEditor.ts"
  );
  const handlers = new Map();
  const editor = {
    on: (name, handler) => handlers.set(name, handler),
    off: (name) => handlers.delete(name),
  };
  const form = document.createElement("form");
  const option = document.createElement("button");
  form.append(option);
  let cancellations = 0;
  const cancel = () => cancellations++;
  const container = { current: form };
  // The native type is only used by the real blur owner.
  globalThis.Node = dom.window.Node;
  const { unmount } = renderHook(() =>
    useMentionAdmissionEditor(editor, container, cancel),
  );
  handlers.get("selectionUpdate")({ transaction: { docChanged: false } });
  handlers.get("selectionUpdate")({ transaction: { docChanged: false } });
  assert.equal(
    cancellations,
    2,
    "departure and return are distinct retirements",
  );
  handlers.get("selectionUpdate")({ transaction: { docChanged: true } });
  assert.equal(
    cancellations,
    2,
    "authored text belongs to the existing query update owner",
  );
  handlers.get("transaction")({ transaction: { getMeta: () => true } });
  assert.equal(cancellations, 3);
  handlers.get("blur")({ event: { relatedTarget: option } });
  assert.equal(cancellations, 3);
  handlers.get("blur")({ event: { relatedTarget: null } });
  assert.equal(cancellations, 4);
  unmount();
  assert.equal(cancellations, 5);
  assert.equal(handlers.size, 0);
});

test("default-agent hotkey owns fresh intent after typing or caret retirement, not displayed search", async () => {
  const React = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { useMentionAdmission } = await import("../lib/useMentionAdmission.ts");
  const { useAgentAddressLockPicker } = await import(
    "./useAgentAddressLockPicker.ts"
  );
  const { useAlwaysAddressShortcut } = await import(
    "./useAlwaysAddressShortcut.ts"
  );
  const agent = "b".repeat(64);
  const suggestion = { pubkey: agent, displayName: "Agent", isAgent: true };
  let generation, release;
  let pendingSearch = true,
    displayed = false,
    defer = false,
    denied = false;
  const writes = [],
    requests = [];
  const { result, rerender } = renderHook(() => {
    generation = React.useRef(0);
    const admit = useMentionAdmission({
      generation,
      isPending: () => pendingSearch,
      channelId: "general",
      channelType: "channel",
      currentPubkey: "a".repeat(64),
      agentPubkeys: new Set([agent]),
      personaIds: new Set(),
      revalidate: async (keys, channel, options) => {
        requests.push({ keys, channel, options });
        if (defer)
          await new Promise((resolve) => {
            release = resolve;
          });
        if (denied) throw new Error("revoked");
        return keys;
      },
    });
    const mentions = {
      admitMentionSelection: admit,
      retireMentionSelection: () => generation.current++,
      getDefaultAgentSuggestion: () => suggestion,
      getMentionDisplayName: () => null,
      getDraftMentionRefs: () => [],
      registerMentionPubkey: (label) => label,
      isMentionOpen: displayed,
      suggestions: [suggestion],
      mentionSelectedIndex: 0,
      isInlineMentionSelection: () => false,
      openMentionPicker: () => {},
    };
    const picker = useAgentAddressLockPicker({
      mentions,
      audience: { pubkeys: [], addPubkey: () => writes.push("pin") },
      audienceScope: "general",
      applyAutocompleteEdit: () => writes.push("insert"),
      richText: { getPlainTextAndCursor: () => ({ text: "draft", cursor: 3 }) },
      onPulseAddressLock: () => {},
    });
    return {
      picker,
      shortcut: useAlwaysAddressShortcut({
        enabled: true,
        mentions,
        onOpenPicker: () => {},
        onToggle: picker.toggleAlwaysAddressAgent,
      }),
    };
  });
  const { isMacPlatform } = await import("../../../shared/lib/platform.ts");
  const press = async () =>
    act(async () => {
      result.current.shortcut({
        code: "KeyM",
        ctrlKey: !isMacPlatform(),
        metaKey: isMacPlatform(),
        shiftKey: true,
        preventDefault() {},
      });
    });
  await press();
  assert.deepEqual(writes, ["insert", "pin"]);
  assert.deepEqual(requests[0], {
    keys: [agent],
    channel: "general",
    options: { phase: "prepare", intendedAgentPubkeys: [agent] },
  });
  writes.length = 0;
  generation.current++; // deliberate caret departure without a render
  await press();
  assert.deepEqual(writes, ["insert", "pin"]);
  writes.length = 0;
  displayed = true;
  rerender();
  const before = requests.length;
  await press();
  assert.equal(
    requests.length,
    before,
    "unsettled displayed row cannot be consumed",
  );
  pendingSearch = false;
  generation.current++;
  await press();
  assert.equal(
    requests.length,
    before,
    "retired displayed callback remains rejected",
  );
  displayed = false;
  rerender();
  denied = true;
  await press();
  assert.deepEqual(writes, [], "default identity still needs fresh authority");
  denied = false;
  defer = true;
  for (const retire of [
    () => generation.current++,
    () => document.dispatchEvent(new dom.window.Event("input")),
    () => document.dispatchEvent(new dom.window.Event("focusout")),
    () => window.dispatchEvent(new dom.window.Event("blur")),
  ]) {
    await press();
    retire();
    await act(async () => release());
    assert.deepEqual(
      writes,
      [],
      "genuine pending departure cancels default pin",
    );
  }
});

test("editor selection refreshes explicit rows without treating owned focus as departure", async () => {
  const { renderHook } = await import("@testing-library/react");
  const { useMentionAdmissionEditor } = await import(
    "../lib/useMentionAdmissionEditor.ts"
  );
  const handlers = new Map();
  const editor = {
    on: (name, handler) => handlers.set(name, handler),
    off: (name) => handlers.delete(name),
  };
  const form = document.createElement("form");
  const option = document.createElement("button");
  form.append(option);
  globalThis.Node = dom.window.Node;
  let departures = 0;
  let selections = 0;
  const cancel = () => departures++;
  const refresh = () => selections++;
  const container = { current: form };
  const { unmount } = renderHook(() =>
    useMentionAdmissionEditor(editor, container, cancel, refresh),
  );
  handlers.get("selectionUpdate")({ transaction: { docChanged: false } });
  assert.equal(selections, 1, "selection has its own row retirement owner");
  assert.equal(departures, 0, "selection is not an external departure");
  handlers.get("blur")({ event: { relatedTarget: option } });
  assert.equal(departures, 0, "owned options retain the tray");
  handlers.get("blur")({ event: { relatedTarget: document.body } });
  assert.equal(departures, 1, "external focus still closes the tray");
  handlers.get("transaction")({ transaction: { getMeta: () => true } });
  assert.equal(departures, 2, "silent restore still closes the tray");
  unmount();
  assert.equal(departures, 3);
});
