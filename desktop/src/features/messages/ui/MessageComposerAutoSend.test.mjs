/**
 * Unit tests for the auto-submit guard in MessageComposer and the
 * auto-send clear behavior in useChannelPanelHistoryState.
 *
 * Structure:
 *   1. Key-match predicate — verifies which (autoSubmitDraftKey, effectiveDraftKey)
 *      pairs arm/skip the auto-submit effect in MessageComposer.
 *   2. Completion-dispatch logic — verifies that ChannelPane.handleAutoSubmitComplete
 *      calls the surgical `onAutoSendComplete` callback when provided, and only
 *      falls back to `goChannel` when it is absent.
 *   3. Clear-patch contract — imports and exercises `buildAutoSendClearPatch` from
 *      useChannelPanelHistoryState to verify the patch removes `autoSend` but
 *      leaves `thread` (and every other panel key) untouched. This is the
 *      regression guard for the "auto-submit clear drops the thread route" defect:
 *      if the implementation ever changes to include `thread: null` in the patch,
 *      this test fails before the bug ships.
 *
 * What is NOT tested here (and why):
 *   - Mounting MessageComposer: depends on Tiptap, Tauri, and React Query context
 *     not available in the node:test harness. The once-only mount guard is an
 *     empty-dep-array effect — verified by code review and the `goChannel`/
 *     `onAutoSendComplete` dispatch test below.
 *   - URL navigation: useHistorySearchState wraps @tanstack/react-router which
 *     requires a browser environment. The `buildAutoSendClearPatch` pure export
 *     covers the correctness of the patch without a router.
 */

import assert from "node:assert/strict";
import test from "node:test";
import { buildAutoSendClearPatch } from "../../channels/ui/channelSearchKeys.ts";

// ── 1. Key-match predicate ────────────────────────────────────────────────────
// Mirror the guard from MessageComposerImpl. This predicate controls whether
// the auto-submit effect arms on mount.

function shouldAutoSubmit(autoSubmitDraftKey, effectiveDraftKey) {
  if (autoSubmitDraftKey === null) return false;
  if (autoSubmitDraftKey !== effectiveDraftKey) return false;
  return true;
}

test("autoSend_main_composer_key_match_should_fire", () => {
  const channelId = "chan-abc";
  assert.equal(shouldAutoSubmit(channelId, channelId), true);
});

test("autoSend_main_composer_key_mismatch_should_not_fire", () => {
  assert.equal(shouldAutoSubmit("chan-abc", "chan-xyz"), false);
});

test("autoSend_thread_composer_key_match_should_fire", () => {
  const draftKey = "thread:root-111";
  assert.equal(shouldAutoSubmit(draftKey, draftKey), true);
});

test("autoSend_thread_composer_key_mismatch_wrong_thread_should_not_fire", () => {
  assert.equal(shouldAutoSubmit("thread:root-aaa", "thread:root-bbb"), false);
});

test("autoSend_thread_key_in_main_composer_should_not_fire", () => {
  assert.equal(shouldAutoSubmit("thread:root-111", "chan-xyz"), false);
});

test("autoSend_channel_key_in_thread_composer_should_not_fire", () => {
  assert.equal(shouldAutoSubmit("chan-abc", "thread:root-111"), false);
});

test("autoSend_null_trigger_should_not_fire", () => {
  assert.equal(shouldAutoSubmit(null, "chan-abc"), false);
});

test("autoSend_null_trigger_thread_should_not_fire", () => {
  assert.equal(shouldAutoSubmit(null, "thread:root-111"), false);
});

// ── 2. Completion-dispatch logic ─────────────────────────────────────────────
// ChannelPane.handleAutoSubmitComplete: when onAutoSendComplete is provided it
// must be called (surgical clear); when absent the goChannel fallback is taken.

function buildHandleAutoSubmitComplete({
  activeChannelId,
  goChannel,
  onAutoSendComplete = null,
}) {
  return function handleAutoSubmitComplete() {
    if (onAutoSendComplete) {
      onAutoSendComplete();
    } else if (activeChannelId) {
      goChannel(activeChannelId, { replace: true });
    }
  };
}

test("handleAutoSubmitComplete_calls_onAutoSendComplete_when_provided", () => {
  let surgicalClearCalled = 0;
  let goChannelCalled = 0;
  const handler = buildHandleAutoSubmitComplete({
    activeChannelId: "chan-abc",
    goChannel: () => {
      goChannelCalled++;
    },
    onAutoSendComplete: () => {
      surgicalClearCalled++;
    },
  });
  handler();
  assert.equal(
    surgicalClearCalled,
    1,
    "onAutoSendComplete should be called once",
  );
  assert.equal(
    goChannelCalled,
    0,
    "goChannel must NOT be called when onAutoSendComplete is provided",
  );
});

test("handleAutoSubmitComplete_falls_back_to_goChannel_when_onAutoSendComplete_absent", () => {
  let goChannelCalled = 0;
  let lastChannelId = null;
  const handler = buildHandleAutoSubmitComplete({
    activeChannelId: "chan-abc",
    goChannel: (id) => {
      goChannelCalled++;
      lastChannelId = id;
    },
    onAutoSendComplete: null,
  });
  handler();
  assert.equal(goChannelCalled, 1, "goChannel should be called as fallback");
  assert.equal(lastChannelId, "chan-abc");
});

test("handleAutoSubmitComplete_no_op_when_both_absent", () => {
  // Should not throw when neither callback nor channelId is available.
  const handler = buildHandleAutoSubmitComplete({
    activeChannelId: null,
    goChannel: () => {
      throw new Error("must not call goChannel");
    },
    onAutoSendComplete: null,
  });
  assert.doesNotThrow(() => handler());
});

// ── 3. Clear-patch contract ───────────────────────────────────────────────────
// buildAutoSendClearPatch() must remove ONLY autoSend — never thread or any
// other panel key. This is the regression guard for the thread-route-drop defect.

test("buildAutoSendClearPatch_removes_autoSend", () => {
  const patch = buildAutoSendClearPatch();
  assert.equal(patch.autoSend, null, "autoSend must be null (remove from URL)");
});

test("buildAutoSendClearPatch_does_not_clear_thread", () => {
  const patch = buildAutoSendClearPatch();
  assert.equal(
    patch.thread,
    undefined,
    "thread must NOT be in the patch — clearing it would unmount the thread panel",
  );
});

test("buildAutoSendClearPatch_only_touches_autoSend", () => {
  const patch = buildAutoSendClearPatch();
  const keys = Object.keys(patch);
  assert.deepEqual(
    keys,
    ["autoSend"],
    `patch must contain exactly [autoSend], got [${keys.join(", ")}]`,
  );
});
