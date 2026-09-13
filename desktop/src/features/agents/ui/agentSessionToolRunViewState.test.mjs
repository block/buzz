import assert from "node:assert/strict";
import test from "node:test";

import {
  initialToolRunGroupViewState,
  reconcileToolRunGroupViewState,
  setToolRunGroupExpanded,
  setToolRunGroupInternalStepsShown,
  setToolRunGroupItemExpanded,
} from "./agentSessionToolRunViewState.ts";

test("a group that mounts already finished starts collapsed", () => {
  assert.equal(initialToolRunGroupViewState("completed").expanded, false);
});

test("a group that mounts live or failed starts open", () => {
  assert.equal(initialToolRunGroupViewState("executing").expanded, true);
  assert.equal(initialToolRunGroupViewState("pending").expanded, true);
  assert.equal(initialToolRunGroupViewState("failed").expanded, true);
});

test("an untouched group auto-collapses when it finishes cleanly", () => {
  const running = initialToolRunGroupViewState("executing");
  const settled = reconcileToolRunGroupViewState(
    running,
    "executing",
    "completed",
  );
  assert.equal(settled.expanded, false);
  assert.equal(settled.userInteracted, false);
});

test("a group the reader opened is never auto-collapsed", () => {
  const touched = setToolRunGroupExpanded(
    initialToolRunGroupViewState("executing"),
    true,
  );
  const settled = reconcileToolRunGroupViewState(
    touched,
    "executing",
    "completed",
  );
  assert.equal(settled.expanded, true);
  assert.equal(settled, touched);
});

test("a group the reader deliberately closed stays closed when it fails", () => {
  const closed = setToolRunGroupExpanded(
    initialToolRunGroupViewState("executing"),
    false,
  );
  const failed = reconcileToolRunGroupViewState(closed, "executing", "failed");
  assert.equal(failed.expanded, false);
});

test("an untouched group opens itself when it starts failing", () => {
  const running = initialToolRunGroupViewState("pending");
  const collapsed = reconcileToolRunGroupViewState(
    running,
    "pending",
    "completed",
  );
  assert.equal(collapsed.expanded, false);
  const failed = reconcileToolRunGroupViewState(
    collapsed,
    "completed",
    "failed",
  );
  assert.equal(failed.expanded, true);
});

test("an unchanged status never moves the group", () => {
  const state = initialToolRunGroupViewState("executing");
  assert.equal(
    reconcileToolRunGroupViewState(state, "executing", "executing"),
    state,
  );
});

test("a group that was never active does not collapse on completed", () => {
  const state = initialToolRunGroupViewState("failed");
  const next = reconcileToolRunGroupViewState(state, "failed", "completed");
  assert.equal(next.expanded, true);
});

test("opening a child step opens the group and latches interaction", () => {
  const state = setToolRunGroupItemExpanded(
    initialToolRunGroupViewState("completed"),
    "tool:b",
    true,
  );
  assert.equal(state.expanded, true);
  assert.equal(state.userInteracted, true);
  assert.deepEqual(state.expandedItemIds, ["tool:b"]);
});

test("closing a child step leaves the group open but drops the pin", () => {
  const opened = setToolRunGroupItemExpanded(
    initialToolRunGroupViewState("completed"),
    "tool:b",
    true,
  );
  const closed = setToolRunGroupItemExpanded(opened, "tool:b", false);
  assert.equal(closed.expanded, true);
  assert.deepEqual(closed.expandedItemIds, []);
});

test("expanded child ids are deterministic regardless of click order", () => {
  const base = initialToolRunGroupViewState("completed");
  const forward = setToolRunGroupItemExpanded(
    setToolRunGroupItemExpanded(base, "tool:a", true),
    "tool:b",
    true,
  );
  const reverse = setToolRunGroupItemExpanded(
    setToolRunGroupItemExpanded(base, "tool:b", true),
    "tool:a",
    true,
  );
  assert.deepEqual(forward.expandedItemIds, reverse.expandedItemIds);
});

test("revealing internal steps latches interaction", () => {
  const state = setToolRunGroupInternalStepsShown(
    initialToolRunGroupViewState("completed"),
    true,
  );
  assert.equal(state.showInternalSteps, true);
  assert.equal(state.userInteracted, true);
  const settled = reconcileToolRunGroupViewState(
    state,
    "executing",
    "completed",
  );
  assert.equal(settled.showInternalSteps, true);
});
