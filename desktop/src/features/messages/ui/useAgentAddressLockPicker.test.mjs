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
});

test("selecting an already addressed agent removes the query and pulses its badge", async () => {
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
    getPlainTextAndCursor: () => ({ text: "ping @", cursor: 6 }),
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
    { replaceFromOffset: 5, replaceToOffset: 6, insertText: "" },
  ]);
  assert.deepEqual(addedPubkeys, []);
  assert.deepEqual(pulsedPubkeys, ["agent-pubkey"]);
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
