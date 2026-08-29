/**
 * REG-16 hardening: proof that ChannelTaskList's DERIVED ranking signals reach
 * the ranker and change the suggested order.
 *
 * Why this file exists. `ownerSuggestion.test.mjs` proves the pure ranker
 * honours `recentParticipantPubkeys` / `openTaskCountByPubkey`, and
 * `SuggestedOwners.test.mjs` proves the rendered order reaches the DOM. Neither
 * could catch the real defect: `ChannelTaskList` rendered `<SuggestedOwners>`
 * with ONLY `candidates` and `task`, so both signals were permanently
 * `undefined` in production. Every unit test passed while the feature was dead.
 * This test mounts the REAL list component over a mocked Tauri boundary, so it
 * fails if the props are ever dropped again.
 *
 * Fixture is built so the two orders provably differ:
 *
 *   candidates (insertion order)      = [ALICE, CAROL, BOB]
 *   recentParticipantPubkeys derived  = [ALICE, CAROL, BOB]
 *   openTaskCountByPubkey derived     = { CAROL: 2 }   (BOB/ALICE hold none)
 *
 *   WITHOUT the signals: ALICE is demoted as task author (+4), CAROL and BOB
 *   tie at 0, so insertion order breaks the tie => CAROL, BOB, ALICE.
 *   WITH the signals: BOB is recent AND unloaded (-3.8), CAROL is recent but
 *   already holds 2 open tasks (-0.9), ALICE is the author (0)
 *   => BOB, CAROL, ALICE.
 *
 * Asserting BOB leads is therefore a direct assertion that the wiring is live;
 * it is exactly the assertion that fails on the pre-fix code.
 */
import assert from "node:assert/strict";
import { after, before, describe, it } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CAROL = "c".repeat(64);

/**
 * The unassigned task under test (t1) plus the history that produces the
 * signals. CAROL is deliberately the assignee of two OPEN tasks and t-done is
 * DONE so it must NOT count toward her workload.
 */
const TASKS = [
  {
    id: "t1",
    channelId: "chan-1",
    title: "Ship the relay migration",
    status: "open",
    assignee: null,
    createdBy: ALICE,
    updatedAt: 100,
  },
  {
    id: "t2",
    channelId: "chan-1",
    title: "Carol busy one",
    status: "open",
    assignee: CAROL,
    createdBy: CAROL,
    updatedAt: 90,
  },
  {
    id: "t3",
    channelId: "chan-1",
    title: "Carol busy two",
    status: "open",
    assignee: CAROL,
    createdBy: CAROL,
    updatedAt: 80,
  },
  {
    id: "t4",
    channelId: "chan-1",
    title: "Bob did this one",
    status: "open",
    assignee: null,
    createdBy: BOB,
    updatedAt: 70,
  },
  {
    id: "t-done",
    channelId: "chan-1",
    title: "Bob finished this",
    status: "done",
    assignee: BOB,
    createdBy: BOB,
    updatedAt: 60,
  },
];

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command) => {
    if (command === "tasks_list") {
      return Promise.resolve(TASKS);
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  },
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

let React, act, createRoot, QueryClient, QueryClientProvider, ChannelTaskList;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ ChannelTaskList } = await import("./ChannelTaskList.tsx"));
});

after(() => dom.window.close());

function makeQueryClient() {
  return new QueryClient({
    defaultOptions: {
      mutations: { gcTime: 0, retry: false },
      queries: { gcTime: 0, retry: false },
    },
  });
}

async function mountList() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const queryClient = makeQueryClient();

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(ChannelTaskList, { channelId: "chan-1" }),
      ),
    );
  });
  // Let the react-query fetch (dynamic import + async invoke) settle.
  await act(async () => {
    await new Promise((r) => setTimeout(r, 50));
  });

  return { container, root, queryClient };
}

describe("ChannelTaskList — derived ranking signals are actually wired", () => {
  it("orders suggestions by recency AND workload, not just the base tier", async () => {
    const { container, root, queryClient } = await mountList();

    const rows = container.querySelectorAll('[data-testid="channel-task-row"]');
    assert.equal(rows.length, TASKS.length, "all tasks render");

    const panels = container.querySelectorAll(
      '[data-testid="suggested-owners"]',
    );
    assert.equal(
      panels.length,
      2,
      "exactly the two UNASSIGNED tasks (t1, t4) get a suggestion panel",
    );

    const ordered = [
      ...panels[0].querySelectorAll('[data-testid="suggested-owner"]'),
    ].map((button) => button.getAttribute("data-pubkey"));

    assert.deepEqual(
      ordered,
      [BOB, CAROL, ALICE],
      "BOB must lead: recent AND holding zero open tasks. Without the wired " +
        "signals this is [CAROL, BOB, ALICE], so this assertion is what " +
        "proves recentParticipantPubkeys/openTaskCountByPubkey are passed.",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("a DONE task does not count toward its assignee's workload", async () => {
    const { container, root, queryClient } = await mountList();

    const panels = container.querySelectorAll(
      '[data-testid="suggested-owners"]',
    );
    const ordered = [
      ...panels[0].querySelectorAll('[data-testid="suggested-owner"]'),
    ].map((button) => button.getAttribute("data-pubkey"));

    // BOB is the assignee of `t-done`. If completed tasks were counted as
    // workload he would be penalised (+1) and could not lead. He leads, so
    // the isTaskDone() guard in the derivation is proven live.
    assert.equal(
      ordered[0],
      BOB,
      "completed tasks must not be counted as open workload",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });

  it("an already-assigned task renders no suggestion panel (FM1)", async () => {
    const { container, root, queryClient } = await mountList();

    const assignedRow = [
      ...container.querySelectorAll('[data-testid="channel-task-row"]'),
    ].find((row) => row.getAttribute("data-task-id") === "t2");
    assert.ok(assignedRow, "the assigned task renders");
    assert.equal(
      assignedRow.parentElement.querySelector(
        '[data-testid="suggested-owners"]',
      ),
      null,
      "no suggestions for a task that already has an owner",
    );

    await act(async () => {
      root.unmount();
    });
    container.remove();
    queryClient.clear();
  });
});
