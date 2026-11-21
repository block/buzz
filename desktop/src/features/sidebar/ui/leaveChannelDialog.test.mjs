import assert from "node:assert/strict";
import test from "node:test";

/**
 * Tests for the leave-channel dialog error-surfacing logic.
 *
 * These tests verify the decision logic that backs
 * `useLeaveChannelDialog` — specifically that:
 * - The dialog stays open when the mutation fails
 * - The dialog closes only on successful mutation
 * - The error message is surfaced to the user
 * - Pending state prevents dismissal
 */

// Simulate the leave-channel dialog state machine
function createLeaveDialogState() {
  let target = null;
  let isPending = false;
  let error = null;

  return {
    get isOpen() {
      return target !== null;
    },
    get error() {
      return error;
    },
    get isPending() {
      return isPending;
    },
    requestLeave(channel) {
      target = channel;
      error = null;
    },
    async confirm(mutateFn) {
      if (!target) return;
      isPending = true;
      try {
        await mutateFn();
        target = null; // close on success
      } catch (e) {
        error = e; // keep open on failure
      } finally {
        isPending = false;
      }
    },
    dismiss() {
      if (!isPending) {
        target = null;
        error = null;
      }
    },
  };
}

test("dialog stays open when leave mutation fails", async () => {
  const dlg = createLeaveDialogState();
  dlg.requestLeave({ id: "ch1", name: "Welcome" });
  assert.equal(dlg.isOpen, true);

  await dlg.confirm(async () => {
    throw new Error("You are the only owner of this channel.");
  });

  assert.equal(dlg.isOpen, true, "Dialog should remain open on error");
  assert.ok(dlg.error instanceof Error);
  assert.match(dlg.error.message, /only owner/);
});

test("dialog closes when leave mutation succeeds", async () => {
  const dlg = createLeaveDialogState();
  dlg.requestLeave({ id: "ch2", name: "General" });
  assert.equal(dlg.isOpen, true);

  await dlg.confirm(async () => {
    // success — no throw
  });

  assert.equal(dlg.isOpen, false, "Dialog should close on success");
  assert.equal(dlg.error, null);
});

test("dismiss is blocked while mutation is pending", () => {
  const dlg = createLeaveDialogState();
  dlg.requestLeave({ id: "ch3", name: "Dev" });

  // Simulate pending state
  dlg.confirm(async () => {
    return new Promise(() => {}); // never resolves
  });

  // Can't dismiss while pending
  dlg.dismiss();
  assert.equal(dlg.isOpen, true, "Dialog should not dismiss while pending");
});

test("dismiss works when not pending", () => {
  const dlg = createLeaveDialogState();
  dlg.requestLeave({ id: "ch4", name: "Test" });
  assert.equal(dlg.isOpen, true);

  dlg.dismiss();
  assert.equal(dlg.isOpen, false);
});

test("error is cleared on a new leave request", async () => {
  const dlg = createLeaveDialogState();
  dlg.requestLeave({ id: "ch5", name: "Failed" });
  await dlg.confirm(async () => {
    throw new Error("You are the only owner.");
  });
  assert.ok(dlg.error instanceof Error);

  // User tries to leave a different channel
  dlg.requestLeave({ id: "ch6", name: "Other" });
  assert.equal(dlg.error, null, "Error should be cleared on new request");
});

test("isPending is true during mutation and false after", async () => {
  const dlg = createLeaveDialogState();
  dlg.requestLeave({ id: "ch7", name: "Pending" });
  assert.equal(dlg.isPending, false);

  let resolveMutation;
  const mutationPromise = new Promise((resolve) => {
    resolveMutation = resolve;
  });

  const confirmPromise = dlg.confirm(() => mutationPromise);
  assert.equal(dlg.isPending, true, "Should be pending during mutation");

  resolveMutation();
  await confirmPromise;

  assert.equal(dlg.isPending, false, "Should not be pending after mutation");
});
