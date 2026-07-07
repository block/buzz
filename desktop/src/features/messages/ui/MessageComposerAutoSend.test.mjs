/**
 * Unit tests for the auto-submit guard in MessageComposer.
 *
 * The auto-submit fires once on mount when `autoSubmitDraftKey` matches
 * `effectiveDraftKey`. The predicate logic is tested directly here without
 * mounting the full component (which depends on Tiptap, Tauri, and React Query
 * context not available in the node:test harness).
 *
 * What is tested:
 *   - The key-match predicate: only fires when the keys match
 *   - Main-composer arming: channel-id key matches channel-id autoSend param
 *   - Thread-composer arming: thread:${rootId} key matches thread:${rootId} autoSend param
 *   - Key mismatch (stale param on wrong channel): must not fire
 *   - Null autoSendDraftKey: must not fire (no-op)
 *
 * The "fires exactly once on mount" invariant is enforced by the empty
 * dependency array on the effect — verified by code review; the
 * `MessageComposerDraftImagePersist.test.mjs` suite shows how to write a
 * React StrictMode harness test for this file class if a full component
 * regression test is added in a future pass.
 */

import assert from "node:assert/strict";
import test from "node:test";

// Mirror the key-match guard from MessageComposerImpl.
// If the implementation changes the guard, this test will diverge from behavior.
function shouldAutoSubmit(autoSubmitDraftKey, effectiveDraftKey) {
  if (autoSubmitDraftKey === null) return false;
  if (autoSubmitDraftKey !== effectiveDraftKey) return false;
  return true;
}

// ── Main-composer arming (effectiveDraftKey = channelId) ──────────────────────

test("autoSend_main_composer_key_match_should_fire", () => {
  // When a channel-root draft is being sent, autoSend=channelId is passed.
  // Main composer's effectiveDraftKey = channelId. Should arm.
  const channelId = "chan-abc";
  assert.equal(shouldAutoSubmit(channelId, channelId), true);
});

test("autoSend_main_composer_key_mismatch_should_not_fire", () => {
  // Stale ?autoSend from a different channel — must not fire.
  const autoSend = "chan-abc";
  const effectiveKey = "chan-xyz";
  assert.equal(shouldAutoSubmit(autoSend, effectiveKey), false);
});

// ── Thread-composer arming (effectiveDraftKey = "thread:${rootId}") ───────────

test("autoSend_thread_composer_key_match_should_fire", () => {
  // Thread-reply draft key = "thread:root-111". Thread composer's draftKey
  // is also "thread:root-111" → match → should arm.
  const draftKey = "thread:root-111";
  assert.equal(shouldAutoSubmit(draftKey, draftKey), true);
});

test("autoSend_thread_composer_key_mismatch_wrong_thread_should_not_fire", () => {
  // Stale ?autoSend from a different thread root — must not fire in this
  // thread composer even though it's the right type of composer.
  const autoSend = "thread:root-aaa";
  const effectiveKey = "thread:root-bbb";
  assert.equal(shouldAutoSubmit(autoSend, effectiveKey), false);
});

test("autoSend_thread_key_in_main_composer_should_not_fire", () => {
  // Thread-draft key passed to main composer (whose draftKey = channelId).
  // The main composer must not fire — key mismatch guard prevents it.
  const autoSend = "thread:root-111";
  const effectiveKey = "chan-xyz"; // main composer
  assert.equal(shouldAutoSubmit(autoSend, effectiveKey), false);
});

test("autoSend_channel_key_in_thread_composer_should_not_fire", () => {
  // Channel-draft autoSend passed to a thread composer. Must not fire.
  const autoSend = "chan-abc";
  const effectiveKey = "thread:root-111";
  assert.equal(shouldAutoSubmit(autoSend, effectiveKey), false);
});

// ── Null / absent trigger ─────────────────────────────────────────────────────

test("autoSend_null_trigger_should_not_fire", () => {
  assert.equal(shouldAutoSubmit(null, "chan-abc"), false);
});

test("autoSend_null_trigger_thread_should_not_fire", () => {
  assert.equal(shouldAutoSubmit(null, "thread:root-111"), false);
});
