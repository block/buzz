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

const AGENT_PUBKEY = "a".repeat(64);
const THREAD_HEAD_ID = "thread-head-1";
const OTHER_THREAD_HEAD_ID = "thread-head-2";

/**
 * Renders the real hook over a recording state harness that re-renders on every
 * write, the way the panel state hooks do — the open handlers read the current
 * thread/profile state as props, so a plain mutable object would hand them stale
 * values and the breadcrumb assertions would silently pass for the wrong reason.
 *
 * Last-opened-wins is a property of these two handlers clearing each other's
 * state, so the assertions are on the state writes, not on a rendered surface.
 */
async function renderAgentSessionHandlers({ openThreadHeadId = null } = {}) {
  const React = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { useChannelAgentSessions } = await import(
    "./useChannelAgentSessions.ts"
  );

  const state = {
    channelManagementOpen: false,
    openAgentSessionChannelId: null,
    openAgentSessionPubkey: null,
    openThreadHeadId,
    profilePanelPubkey: null,
  };
  const openedThreads = [];
  let commit = () => {};
  const write = (key, value) => {
    state[key] = value;
    commit();
  };

  const rendered = renderHook(() => {
    const [, force] = React.useState(0);
    commit = () => force((version) => version + 1);

    return useChannelAgentSessions({
      activeChannel: { id: "channel-1", name: "general" },
      activeChannelId: "channel-1",
      agentsLoaded: true,
      channelMembers: [{ pubkey: AGENT_PUBKEY, role: "bot" }],
      handleOpenThread: (message) => {
        openedThreads.push(message.id);
        write("openThreadHeadId", message.id);
      },
      managedAgents: [
        {
          agentSource: "managed",
          canInterruptTurn: true,
          name: "ss-dev-00",
          pubkey: AGENT_PUBKEY,
          status: "deployed",
        },
      ],
      openAgentSessionPubkey: state.openAgentSessionPubkey,
      openThreadHeadId: state.openThreadHeadId,
      profilePanelPubkey: state.profilePanelPubkey,
      setChannelManagementOpen: (open) => write("channelManagementOpen", open),
      setExpandedThreadReplyIds: () => {},
      setOpenAgentSessionChannelId: (value) =>
        write("openAgentSessionChannelId", value),
      setOpenAgentSessionPubkey: (value) =>
        write("openAgentSessionPubkey", value),
      setOpenThreadHeadId: (value) => write("openThreadHeadId", value),
      setProfilePanelPubkey: (value) => write("profilePanelPubkey", value),
      setThreadReplyTargetId: () => {},
      setThreadScrollTargetId: () => {},
    });
  });

  const run = (body) => act(async () => body(rendered.result.current));

  return { openedThreads, run, state };
}

test("opening activity over a thread clears the thread", async () => {
  const { run, state } = await renderAgentSessionHandlers({
    openThreadHeadId: THREAD_HEAD_ID,
  });

  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));

  assert.equal(state.openAgentSessionPubkey, AGENT_PUBKEY);
  assert.equal(state.openThreadHeadId, null);
});

test("opening a thread over activity clears the agent session", async () => {
  const { openedThreads, run, state } = await renderAgentSessionHandlers();
  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));

  await run((handlers) =>
    handlers.openThreadAndCloseAgentSession({ id: THREAD_HEAD_ID }),
  );

  assert.equal(state.openAgentSessionPubkey, null);
  assert.deepEqual(openedThreads, [THREAD_HEAD_ID]);
  assert.equal(state.openThreadHeadId, THREAD_HEAD_ID);
});

test("either ordering ends with exactly one surface's state live", async () => {
  // Both directions back to back with no close in between: whichever handler ran
  // last is the only one holding state, so the surface resolver never sees two
  // candidates and its priority tie-break never has to decide.
  const { run, state } = await renderAgentSessionHandlers();

  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));
  await run((handlers) =>
    handlers.openThreadAndCloseAgentSession({ id: THREAD_HEAD_ID }),
  );

  assert.equal(state.openAgentSessionPubkey, null);
  assert.equal(state.openThreadHeadId, THREAD_HEAD_ID);

  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));

  assert.equal(state.openAgentSessionPubkey, AGENT_PUBKEY);
  assert.equal(state.openThreadHeadId, null);
});

test("back from activity opened over a thread returns to that thread", async () => {
  // The replacement clears the thread, so the breadcrumb is what keeps it
  // recoverable; without it, last-opened-wins would be a one-way door.
  const { run, state } = await renderAgentSessionHandlers({
    openThreadHeadId: THREAD_HEAD_ID,
  });

  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));
  await run((handlers) => handlers.backFromAgentSession());

  assert.equal(state.openAgentSessionPubkey, null);
  assert.equal(state.openThreadHeadId, THREAD_HEAD_ID);
});

test("back never resurrects a thread from an earlier replacement", async () => {
  // Alternating both directions leaves one breadcrumb, not a stack:
  // `openThreadAndCloseAgentSession` clears the recorded target, so the next
  // activity open records the thread actually on screen.
  const { run, state } = await renderAgentSessionHandlers({
    openThreadHeadId: THREAD_HEAD_ID,
  });

  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));
  await run((handlers) =>
    handlers.openThreadAndCloseAgentSession({ id: OTHER_THREAD_HEAD_ID }),
  );
  await run((handlers) => handlers.openAgentSession(AGENT_PUBKEY, "channel-1"));
  await run((handlers) => handlers.backFromAgentSession());

  assert.equal(state.openThreadHeadId, OTHER_THREAD_HEAD_ID);
});
