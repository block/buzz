import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

import {
  buildMentionWakePlan,
  hasSubstantiveNonMentionText,
  MENTION_WAKE_GATE_HOLD_MS,
  useMentionWakePreflight,
} from "./useMentionWakePreflight.ts";

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

const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);

const fizzRef = { displayName: "Fizz", pubkey: AGENT, isAgent: true };

const localAgentOptions = (contentRef, onStart, overrides = {}) => ({
  channelId: "general",
  contentRef,
  enabled: true,
  expectedRelayUrl: "wss://relay.example",
  expectedSignerPubkey: "c".repeat(64),
  getDraftMentionRefs: () => [fizzRef],
  getManagedAgentsByPubkey: async () =>
    new Map([
      [AGENT, { pubkey: AGENT, status: "stopped", backend: { type: "local" } }],
    ]),
  isManagedAgentPubkey: () => true,
  memberPubkeys: new Set([AGENT]),
  startManagedAgent: async () => {
    onStart();
    return { pubkey: AGENT, status: "running" };
  },
  ...overrides,
});

// Suspends the managed-agent lookup so a matured wake can be held mid-flight,
// which is the window in which a cancelled arming could resurrect itself.
const suspendedLookup = () => {
  let resolve;
  const lookup = new Promise((r) => {
    resolve = r;
  });
  return {
    getManagedAgentsByPubkey: () => lookup,
    settle: async () => {
      resolve(
        new Map([
          [
            AGENT,
            { pubkey: AGENT, status: "stopped", backend: { type: "local" } },
          ],
        ]),
      );
      await lookup;
    },
  };
};

test("mention-only composer content is not substantive", () => {
  assert.equal(hasSubstantiveNonMentionText("@Fizz ", [fizzRef]), false);
  assert.equal(hasSubstantiveNonMentionText("@Fizz hello", [fizzRef]), true);
  assert.equal(hasSubstantiveNonMentionText("hello @Fizz", [fizzRef]), true);
});

test("wake plan contains only managed agents already in the channel", () => {
  const plan = buildMentionWakePlan({
    channelId: "general",
    content: "@Fizz ask @Imp for help",
    isManagedAgentPubkey: (pubkey) => pubkey === AGENT,
    memberPubkeys: new Set([AGENT, OTHER_AGENT]),
    mentionRefs: [
      fizzRef,
      { displayName: "Imp", pubkey: OTHER_AGENT, isAgent: true },
    ],
  });

  assert.deepEqual(plan, {
    key: `general:${AGENT}`,
    pubkeys: [AGENT],
  });
});

test("wake plan rejects non-member managed agents", () => {
  assert.equal(
    buildMentionWakePlan({
      channelId: "general",
      content: "@Fizz hello",
      isManagedAgentPubkey: () => true,
      memberPubkeys: new Set(),
      mentionRefs: [fizzRef],
    }),
    null,
  );
});

test("editor updates arm a mention-first draft without a rerender", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  let starts = 0;
  const contentRef = { current: "@Fizz " };
  const view = renderHook(() =>
    useMentionWakePreflight({
      channelId: "general",
      contentRef,
      enabled: true,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: "c".repeat(64),
      getDraftMentionRefs: () => [fizzRef],
      getManagedAgentsByPubkey: async () =>
        new Map([
          [
            AGENT,
            { pubkey: AGENT, status: "stopped", backend: { type: "local" } },
          ],
        ]),
      isManagedAgentPubkey: () => true,
      memberPubkeys: new Set([AGENT]),
      startManagedAgent: async () => {
        starts += 1;
        return { pubkey: AGENT, status: "running" };
      },
    }),
  );

  contentRef.current = "@Fizz please investigate";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  assert.equal(starts, 1);
});

test("typing through the window does not restart the gate hold", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  let starts = 0;
  const contentRef = { current: "@Fizz " };
  const view = renderHook(() =>
    useMentionWakePreflight(
      localAgentOptions(contentRef, () => {
        starts += 1;
      }),
    ),
  );

  contentRef.current = "@Fizz initial";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS / 2));
  assert.equal(starts, 0);

  // Still typing to the same agent: no gate lapsed, so the window keeps running
  // from its original arming. The wake fires one second after the gates began
  // holding, not one second after the last keystroke.
  contentRef.current = "@Fizz initial and still typing";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS / 2));

  assert.equal(starts, 1);
});

test("a lapse in the gates requires a fresh full gate hold", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  let starts = 0;
  const contentRef = { current: "@Fizz " };
  const view = renderHook(() =>
    useMentionWakePreflight(
      localAgentOptions(contentRef, () => {
        starts += 1;
      }),
    ),
  );

  contentRef.current = "@Fizz initial";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS - 100));

  // Trimmed back to the bare mention: the substantive-text gate stops holding,
  // which cancels the window outright instead of letting it mature 100 ms later.
  contentRef.current = "@Fizz ";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));
  assert.equal(starts, 0);

  contentRef.current = "@Fizz back again";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS - 1));
  assert.equal(starts, 0);
  await act(async () => t.mock.timers.tick(1));

  assert.equal(starts, 1);
});

test("a trimmed-then-retyped draft cannot resurrect the cancelled wake", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  const lookup = suspendedLookup();
  let starts = 0;
  const contentRef = { current: "@Fizz initial" };
  const view = renderHook(() =>
    useMentionWakePreflight(
      localAgentOptions(
        contentRef,
        () => {
          starts += 1;
        },
        { getManagedAgentsByPubkey: lookup.getManagedAgentsByPubkey },
      ),
    ),
  );

  // Mature the hold, then park the arming inside its managed-agent lookup.
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  // Trimming to the bare mention lapses the substantive-text gate and cancels
  // that arming; retyping restores an identical plan key within the same round
  // trip. Only the arming's identity separates the dead window from the new one.
  contentRef.current = "@Fizz ";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  contentRef.current = "@Fizz back again";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(1));
  await act(lookup.settle);
  assert.equal(starts, 0);

  // The replacement window is still live, so this is a fence, not a mute.
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  assert.equal(starts, 1);
});

test("removing and re-adding the mention cannot resurrect the cancelled wake", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  const lookup = suspendedLookup();
  let starts = 0;
  const contentRef = { current: "@Fizz initial" };
  const view = renderHook(() =>
    useMentionWakePreflight(
      localAgentOptions(
        contentRef,
        () => {
          starts += 1;
        },
        { getManagedAgentsByPubkey: lookup.getManagedAgentsByPubkey },
      ),
    ),
  );

  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  // Same race, reached down the other cancellation path: dropping the mention
  // entirely nulls the plan on the "@"-free short-circuit rather than on the
  // substantive-text gate.
  contentRef.current = "please investigate";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  contentRef.current = "@Fizz back again";
  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(1));
  await act(lookup.settle);
  assert.equal(starts, 0);

  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  assert.equal(starts, 1);
});

test("navigating away and back cannot resurrect the cancelled wake", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  const lookup = suspendedLookup();
  let starts = 0;
  const contentRef = { current: "@Fizz please investigate" };
  const view = renderHook(
    ({ channelId }) =>
      useMentionWakePreflight(
        localAgentOptions(
          contentRef,
          () => {
            starts += 1;
          },
          {
            channelId,
            getManagedAgentsByPubkey: lookup.getManagedAgentsByPubkey,
          },
        ),
      ),
    { initialProps: { channelId: "general" } },
  );

  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  // The plan key carries the channel, so navigating away cancels the in-flight
  // arming and navigating back hands the same key to a new one.
  await act(async () => view.rerender({ channelId: "random" }));
  await act(async () => view.rerender({ channelId: "general" }));
  await act(lookup.settle);
  assert.equal(starts, 0);

  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  assert.equal(starts, 1);
});

test("mention-free drafts skip the mention-ref snapshot entirely", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  let snapshots = 0;
  const contentRef = { current: "" };
  const view = renderHook(() =>
    useMentionWakePreflight({
      channelId: "general",
      contentRef,
      enabled: true,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: "c".repeat(64),
      getDraftMentionRefs: () => {
        snapshots += 1;
        return [];
      },
      getManagedAgentsByPubkey: async () => new Map(),
      isManagedAgentPubkey: () => true,
      memberPubkeys: new Set([AGENT]),
      startManagedAgent: async () => ({ pubkey: AGENT, status: "running" }),
    }),
  );

  contentRef.current = "no mentions in this draft";
  act(() => view.result.current.prepareMentionWake(contentRef.current));

  assert.equal(snapshots, 0);
});

test("a prewake start is marked speculative", async (t) => {
  // The flag is what bounds the spawned harness's lifetime: without it an
  // abandoned draft leaks a live agent until app quit. Nothing else in the app
  // sets it, so this call site is the whole contract.
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  const inputs = [];
  const contentRef = { current: "@Fizz please investigate" };
  const view = renderHook(() =>
    useMentionWakePreflight(
      localAgentOptions(contentRef, () => {}, {
        startManagedAgent: async (input) => {
          inputs.push(input);
          return { pubkey: AGENT, status: "running" };
        },
      }),
    ),
  );

  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  assert.deepEqual(inputs, [
    {
      pubkey: AGENT,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: "c".repeat(64),
      speculative: true,
    },
  ]);
});

test("provider-backed agents are never woken speculatively", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  let starts = 0;
  const contentRef = { current: "@Fizz please investigate" };
  const view = renderHook(() =>
    useMentionWakePreflight({
      channelId: "general",
      contentRef,
      enabled: true,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: "c".repeat(64),
      getDraftMentionRefs: () => [fizzRef],
      getManagedAgentsByPubkey: async () =>
        new Map([
          [
            AGENT,
            {
              pubkey: AGENT,
              status: "not_deployed",
              backend: { type: "provider", id: "blox", config: {} },
            },
          ],
        ]),
      isManagedAgentPubkey: () => true,
      memberPubkeys: new Set([AGENT]),
      startManagedAgent: async () => {
        starts += 1;
        return { pubkey: AGENT, status: "deployed" };
      },
    }),
  );

  act(() => view.result.current.prepareMentionWake(contentRef.current));
  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));

  assert.equal(starts, 0);
});

test("unmount prevents a wake after an in-flight lookup resolves", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { act, renderHook } = await import("@testing-library/react");
  let resolveManagedAgents;
  const managedAgents = new Promise((resolve) => {
    resolveManagedAgents = resolve;
  });
  let starts = 0;
  const contentRef = { current: "@Fizz hello" };
  const view = renderHook(() =>
    useMentionWakePreflight({
      channelId: "general",
      contentRef,
      enabled: true,
      expectedRelayUrl: "wss://relay.example",
      expectedSignerPubkey: "c".repeat(64),
      getDraftMentionRefs: () => [fizzRef],
      getManagedAgentsByPubkey: () => managedAgents,
      isManagedAgentPubkey: () => true,
      memberPubkeys: new Set([AGENT]),
      startManagedAgent: async () => {
        starts += 1;
        return { pubkey: AGENT, status: "running" };
      },
    }),
  );

  await act(async () => t.mock.timers.tick(MENTION_WAKE_GATE_HOLD_MS));
  view.unmount();
  await act(async () => {
    resolveManagedAgents(
      new Map([
        [
          AGENT,
          { pubkey: AGENT, status: "stopped", backend: { type: "local" } },
        ],
      ]),
    );
    await managedAgents;
  });

  assert.equal(starts, 0);
});
