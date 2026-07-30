import assert from "node:assert/strict";
import test from "node:test";

/**
 * Tests for the invite creation + clipboard copy error separation.
 *
 * The old code wrapped both mintInvite and writeTextToClipboard in a
 * single try/catch, making it impossible to tell the user whether the
 * invite failed to create or the clipboard failed to copy.
 *
 * The new code separates the two failure modes so the user gets an
 * accurate error message. These tests verify that decision logic.
 */

// Simulate the handleCopy state machine
function createInviteCopyState() {
  let copyStatus = "idle";
  const toasts = [];

  async function handleCopy(mintInviteFn, clipboardFn) {
    if (copyStatus === "copying") return;
    copyStatus = "copying";
    let invite;
    try {
      invite = await mintInviteFn();
    } catch (error) {
      copyStatus = "idle";
      toasts.push({
        type: "error",
        message: `Couldn't create the invite: ${error instanceof Error ? error.message : "unknown error"}`,
      });
      return;
    }
    try {
      await clipboardFn(invite.url);
      copyStatus = "copied";
      toasts.push({ type: "success", message: "Invite link copied" });
    } catch {
      copyStatus = "idle";
      toasts.push({
        type: "error",
        message: "Invite created, but copying to the clipboard failed.",
      });
    }
  }

  return {
    get copyStatus() { return copyStatus; },
    get toasts() { return toasts; },
    handleCopy,
  };
}

test("both succeed: copies invite and shows success toast", async () => {
  const state = createInviteCopyState();
  await state.handleCopy(
    async () => ({ url: "https://buzz.example/i/abc123" }),
    async () => {},
  );
  assert.equal(state.copyStatus, "copied");
  assert.equal(state.toasts.length, 1);
  assert.equal(state.toasts[0].type, "success");
  assert.match(state.toasts[0].message, /Invite link copied/);
});

test("mintInvite fails: shows creation error, does not attempt clipboard", async () => {
  const state = createInviteCopyState();
  let clipboardCalled = false;
  await state.handleCopy(
    async () => { throw new Error("HTTP 403"); },
    async () => { clipboardCalled = true; },
  );
  assert.equal(state.copyStatus, "idle");
  assert.equal(clipboardCalled, false, "Clipboard should not be called when invite creation fails");
  assert.equal(state.toasts.length, 1);
  assert.equal(state.toasts[0].type, "error");
  assert.match(state.toasts[0].message, /Couldn't create the invite/);
  assert.match(state.toasts[0].message, /HTTP 403/);
});

test("clipboard fails: shows clipboard-specific error, invite was created", async () => {
  const state = createInviteCopyState();
  await state.handleCopy(
    async () => ({ url: "https://buzz.example/i/xyz789" }),
    async () => { throw new Error("Clipboard not available"); },
  );
  assert.equal(state.copyStatus, "idle");
  assert.equal(state.toasts.length, 1);
  assert.equal(state.toasts[0].type, "error");
  assert.match(state.toasts[0].message, /Invite created, but copying/);
});

test("mintInvite error message is surfaced to the user", async () => {
  const state = createInviteCopyState();
  await state.handleCopy(
    async () => { throw new Error("You are not a channel owner."); },
    async () => {},
  );
  assert.match(state.toasts[0].message, /not a channel owner/);
});

test("non-Error thrown by mintInvite produces generic message", async () => {
  const state = createInviteCopyState();
  await state.handleCopy(
    async () => { throw "string error"; },
    async () => {},
  );
  assert.match(state.toasts[0].message, /unknown error/);
});

test("copying status is set during operation", async () => {
  const state = createInviteCopyState();
  let mintResolve;
  const mintPromise = new Promise((resolve) => { mintResolve = resolve; });

  const copyPromise = state.handleCopy(
    () => mintPromise,
    async () => {},
  );
  assert.equal(state.copyStatus, "copying");

  mintResolve({ url: "https://buzz.example/i/test" });
  await copyPromise;

  assert.equal(state.copyStatus, "copied");
});
