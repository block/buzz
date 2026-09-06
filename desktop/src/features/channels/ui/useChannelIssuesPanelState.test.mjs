import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";

import { useChannelIssuesPanelToggle } from "./useChannelIssuesPanelState.ts";

async function withToggle(assertion) {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost",
  });
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  const { act, cleanup, renderHook } = await import("@testing-library/react");

  try {
    const calls = {
      closeAgentSession: 0,
      issues: 0,
      management: [],
      profile: [],
      thread: [],
    };
    const hook = renderHook(() =>
      useChannelIssuesPanelToggle({
        closeAgentSession: () => {
          calls.closeAgentSession += 1;
        },
        setChannelManagementOpen: (open) => calls.management.push(open),
        setOpenThreadHeadId: (id) => calls.thread.push(id),
        setProfilePanelPubkey: (pubkey) => calls.profile.push(pubkey),
        toggleIssues: () => {
          calls.issues += 1;
        },
      }),
    );
    await assertion({ act, calls, hook });
  } finally {
    cleanup();
    dom.window.close();
  }
}

test("non-split issues toggle closes the open thread", async () => {
  await withToggle(async ({ act, calls, hook }) => {
    await act(async () => hook.result.current());

    assert.deepEqual(calls.thread, [null]);
    assert.equal(calls.issues, 1);
  });
});

test("split issues toggle preserves the open thread", async () => {
  await withToggle(async ({ act, calls, hook }) => {
    await act(async () => hook.result.current({ preserveThread: true }));

    assert.deepEqual(calls.thread, []);
    assert.equal(calls.issues, 1);
  });
});
