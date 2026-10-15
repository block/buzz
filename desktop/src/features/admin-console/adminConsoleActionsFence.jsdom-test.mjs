/**
 * Community fences inside one mounted Actions section: when the same section
 * moves to another community with the same target, an unresolved read from
 * the old community never shows in, or unlocks, the new one. Panel navigation
 * unmounts the section and is covered separately in the Actions tests.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  React,
  act,
  createRoot,
  CommunitiesProvider,
  CM_ORIGIN,
  CM_PUBKEY,
  deferred,
  fireEvent,
  makeQueryClient,
  QueryClientProvider,
  resetTestState,
  setIpcHandler,
  settle,
  TEST_COMMUNITY,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";
import {
  ActionsSection,
  AdminMemberPicker,
  DirectActionsProvider,
  useDirectActions,
} from "./AdminConsoleActionsTab.tsx";

afterEach(resetTestState);

const A = TEST_COMMUNITY;
const B = {
  id: "22222222-2222-4222-8222-222222222222",
  host: "beta.example.com",
  icon: null,
};
const TARGET = "ab".repeat(32);
const EVENT = "ef".repeat(32);
const q = (c, id) => c.querySelector(`[data-testid='${id}']`);
const member = {
  pubkey: TARGET,
  displayName: null,
  avatarUrl: null,
  nip05Handle: null,
  ownerPubkey: null,
  isAgent: false,
};
const draft = (host, action) => ({
  host,
  action,
  target: action === "delete" ? EVENT : "",
  member: action === "delete" ? null : member,
  reason: "",
  secs: "",
});

/** Mount `child(community)` once; `show(community)` re-renders the same tree. */
function mountSection(child) {
  setIpcHandler("admin_connected_community_host", () =>
    Promise.resolve(A.host),
  );
  setIpcHandler("get_users_batch", () =>
    Promise.resolve({ profiles: {}, missing: [] }),
  );
  const qc = makeQueryClient(CM_PUBKEY);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const ctl = {};
  function Grab() {
    ctl.current = useDirectActions();
    return null;
  }
  const render = (community) =>
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: qc },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            DirectActionsProvider,
            {
              canMutate: true,
              origin: CM_ORIGIN,
              pubkey: CM_PUBKEY,
              generation: 0,
            },
            React.createElement(Grab),
            child(community),
          ),
        ),
      ),
    );
  const show = async (community) => {
    await act(async () => render(community));
    await settle();
  };
  /** Move the draft and the section to `community` in one render, so the lookup never idles between them. */
  const switchTo = async (community, d) => {
    await act(async () => {
      ctl.current.setDraft(d);
      render(community);
    });
    await settle();
  };
  const setDraft = async (d) => {
    await act(async () => ctl.current.setDraft(d));
    await settle();
  };
  const unmount = async () => {
    await act(async () => root.unmount());
    document.body.removeChild(container);
  };
  return { container, show, setDraft, switchTo, unmount };
}

/** Per-host deferred replies for `cmd`. */
function deferPerHost(cmd) {
  const replies = {};
  setIpcHandler(cmd, ({ communityHost }) => {
    replies[communityHost] = deferred();
    return replies[communityHost].promise;
  });
  return replies;
}

const actions = (community) =>
  React.createElement(ActionsSection, { community });

test("fence-member-same-section: A's late member lookup never shows or unlocks Review in B", async () => {
  // Mutation: drop communityHost from ActionsSection's lookup fence → RED.
  const replies = deferPerHost("admin_get_member");
  const s = mountSection(actions);
  try {
    await s.show(A);
    await s.setDraft(draft(A.host, "ban"));
    assert.ok(replies[A.host], "A's lookup started");
    await s.switchTo(B, draft(B.host, "ban"));
    assert.ok(replies[B.host], "B starts its own lookup");
    await act(async () =>
      replies[A.host].resolve({
        pubkey: TARGET,
        profile: null,
        role: "owner",
        banned: false,
        mutedUntil: null,
        isStaff: false,
      }),
    );
    await settle();
    assert.ok(
      !q(s.container, "direct-member-state"),
      "A's member never shows in B",
    );
    assert.ok(q(s.container, "direct-review-btn").disabled, "B waits for B");
  } finally {
    await s.unmount();
  }
});

test("fence-event-same-section: A's late event preview never shows or unlocks Review in B", async () => {
  // Mutation: drop communityHost from ActionsSection's lookup fence → RED.
  const replies = deferPerHost("admin_get_event");
  const s = mountSection(actions);
  try {
    await s.show(A);
    await s.setDraft(draft(A.host, "delete"));
    assert.ok(replies[A.host], "A's preview started");
    await s.switchTo(B, draft(B.host, "delete"));
    assert.ok(replies[B.host], "B starts its own preview");
    await act(async () =>
      replies[A.host].resolve({
        id: EVENT,
        authorPubkey: TARGET,
        kind: 9,
        content: "content in a",
        createdAt: "2026-09-30T00:00:00Z",
        deletedAt: null,
        channelId: null,
      }),
    );
    await settle();
    assert.doesNotMatch(s.container.textContent, /content in a/);
    assert.ok(q(s.container, "direct-review-btn").disabled, "B waits for B");
  } finally {
    await s.unmount();
  }
});

test("fence-search-same-section: A's late search results never list in B", async () => {
  // Mutation: drop communityHost from the member-search fence → RED.
  const replies = deferPerHost("admin_search_members");
  const picker = (community) =>
    React.createElement(AdminMemberPicker, {
      communityHost: community.host,
      disabled: false,
      member: null,
      onChange: () => {},
    });
  const s = mountSection(picker);
  const hit = (pubkey, displayName) => ({
    pubkey,
    displayName,
    avatarUrl: null,
    nip05: null,
  });
  try {
    await s.show(A);
    await act(async () => {
      fireEvent.change(q(s.container, "direct-member-input"), {
        target: { value: "sam" },
      });
    });
    await settle();
    assert.ok(replies[A.host], "A's search started");
    await s.show(B);
    assert.ok(replies[B.host], "B searches for itself");
    await act(async () =>
      replies[A.host].resolve({ items: [hit("a1".repeat(32), "Sam in A")] }),
    );
    await settle();
    assert.doesNotMatch(s.container.textContent, /Sam in A/);
    await act(async () =>
      replies[B.host].resolve({ items: [hit("b1".repeat(32), "Sam in B")] }),
    );
    await settle();
    assert.match(s.container.textContent, /Sam in B/);
    assert.doesNotMatch(s.container.textContent, /Sam in A/);
  } finally {
    await s.unmount();
  }
});
