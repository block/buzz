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

test("turning off automatic mentions opens a closed mention picker first", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useComposerMentionPicker } = await import(
    "./useComposerMentionPicker.ts"
  );
  const openCalls = [];
  let turnOffCount = 0;
  const { result } = renderHook(() =>
    useComposerMentionPicker({
      mentions: {
        cancelMentionAutocomplete: () => {},
        isMentionOpen: false,
        openMentionPicker: (...args) => openCalls.push(args),
        updateMentionQuery: () => {},
      },
      onTurnOffAutoPinConfirmation: () => {
        turnOffCount += 1;
      },
      richText: {
        editor: {},
        focus: () => {},
        getPlainTextAndCursor: () => ({ cursor: 4, text: "ping" }),
      },
      setIsEmojiPickerOpen: () => {},
    }),
  );

  act(() => result.current.turnOff());

  assert.deepEqual(openCalls, [[4, "first-agent"]]);
  assert.equal(turnOffCount, 1);
});

test("turning off automatic mentions refreshes an open picker without closing it", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { useComposerMentionPicker } = await import(
    "./useComposerMentionPicker.ts"
  );
  let pickerMutationCount = 0;
  let turnOffCount = 0;
  const { result } = renderHook(() =>
    useComposerMentionPicker({
      mentions: {
        cancelMentionAutocomplete: () => {
          pickerMutationCount += 1;
        },
        isMentionOpen: true,
        openMentionPicker: () => {
          pickerMutationCount += 1;
        },
        updateMentionQuery: () => {},
      },
      onTurnOffAutoPinConfirmation: () => {
        turnOffCount += 1;
      },
      richText: {
        editor: {},
        focus: () => {},
        getPlainTextAndCursor: () => ({ cursor: 4, text: "ping" }),
      },
      setIsEmojiPickerOpen: () => {},
    }),
  );

  act(() => result.current.turnOff());

  assert.equal(pickerMutationCount, 1);
  assert.equal(turnOffCount, 1);
});
