/**
 * Shared jsdom lifecycle, item fixtures and the `renderBlock` view helper for
 * the work-block suites. One owner for the order-sensitive globals: the two
 * suites run in separate processes, so a second copy would not collide -- it
 * would drift, which is the failure this rig exists to prevent.
 */

import { after, afterEach, before } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

class NoopObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

let prefersReducedMotion = false;

/**
 * ESM bindings are read-only in importers, so the reduced-motion test cannot
 * assign this directly -- it goes through the setter, and `afterEach` below
 * resets it for every file that imports this rig.
 */
export function setPrefersReducedMotion(value) {
  prefersReducedMotion = value;
}

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    IntersectionObserver: NoopObserver,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    ResizeObserver: NoopObserver,
    getComputedStyle: (...args) => dom.window.getComputedStyle(...args),
    localStorage: dom.window.localStorage,
    self: dom.window,
    window: dom.window,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  // `motion`'s useReducedMotion reads this query, so the reduced-motion test
  // drives it through the same surface the component does.
  dom.window.matchMedia = (query) => ({
    matches:
      prefersReducedMotion && String(query).includes("prefers-reduced-motion"),
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
  });
  globalThis.matchMedia = dom.window.matchMedia;
  dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  dom.window.cancelAnimationFrame = (id) => clearTimeout(id);
  globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
  globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;
});

afterEach(async () => {
  prefersReducedMotion = false;
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

export const START = "2026-06-18T00:00:00.000Z";
export const SHARED = {
  channelId: "chan-1",
  sessionId: "sess-1",
  turnId: "turn-1",
};

export function step(id, overrides = {}) {
  return {
    ...SHARED,
    id,
    type: "tool",
    renderClass: "shell",
    descriptor: {
      renderClass: "shell",
      label: "Ran command",
      preview: id,
      source: "shell",
      groupKey: "shell:command",
    },
    title: id,
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "ok",
    isError: false,
    timestamp: START,
    startedAt: START,
    completedAt: "2026-06-18T00:00:01.000Z",
    ...overrides,
  };
}

export function thoughtStep(id) {
  return {
    ...SHARED,
    id,
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: "weighing the options",
    timestamp: START,
  };
}

export function noteStep(id, text = "posted the summary to the channel") {
  return {
    ...SHARED,
    id,
    type: "message",
    renderClass: "message",
    role: "assistant",
    title: "Agent",
    text,
    timestamp: START,
  };
}

/**
 * A relay post the agent made mid-turn.
 *
 * The distinguishing property is `descriptor.renderClass: "message"`, which is
 * what `buildCompactToolSummary` turns into `presentation: "message"` and what
 * routes the item to the avatar + speech-bubble presenter outside a block. The
 * result carries an `event_id` so `getSentMessageLink` resolves too — that is
 * the fully-featured shape (bubble, timestamp, delivery receipt), i.e. the one
 * that goes most wrong on a muted rail.
 */
export function relayStep(id, overrides = {}) {
  return step(id, {
    renderClass: "message",
    descriptor: {
      renderClass: "message",
      label: "Send Message",
      preview: "posted the findings to the channel",
      action: { verb: "Sent", object: "posted the findings to the channel" },
      source: "shell",
      groupKey: "buzz-cli:messages.send",
      operation: "messages.send",
    },
    title: "Send Message",
    toolName: "buzz_dev_mcp__shell",
    args: {
      command:
        "buzz messages send --channel chan-1 --content 'posted the findings to the channel'",
    },
    result: '{"event_id":"ev-1","accepted":true}',
    ...overrides,
  });
}

/**
 * Let every pending animation finish and the DOM settle.
 *
 * A collapsing block keeps its rail mounted until the height animation's exit
 * completes, which is the point of the fold — so "has it closed yet?" can only
 * be asked after the exit runs, not on the commit that started it. Polling to a
 * stable answer keeps these assertions about the end state the reader sees
 * rather than about motion's internal frame schedule.
 */
export async function settle(read, expected, { timeout = 2000 } = {}) {
  const { act } = await import("@testing-library/react");
  const deadline = Date.now() + timeout;
  let last = read();
  while (Date.now() < deadline) {
    if (last === expected) return last;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 25));
    });
    last = read();
  }
  return last;
}

/**
 * Mount a work block under a router.
 *
 * Rail rows render agent content through `Markdown`, which resolves in-app links
 * via `useAppNavigation` and therefore needs router context — the same reason
 * `AgentSessionTranscriptList.conversation.test.mjs` mounts through a memory
 * router. Without it a thought row throws while a tool row happens not to.
 *
 * `liveTurnId` defaults to the fixtures' own turn, i.e. "the agent is working on
 * this turn right now", because that is the situation nearly every case here is
 * about. Pass `liveTurnId: null` for reopened history — see the orphaned-step
 * tests, where the same items must read as finished.
 */
export async function renderBlock(
  items,
  { liveTurnId = SHARED.turnId, streamingItemId = null } = {},
) {
  const { createElement, useState } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { createMemoryHistory, createRootRoute, createRouter, RouterProvider } =
    await import("@tanstack/react-router");
  const { AgentSessionTranscriptTurnMetaProvider } = await import(
    "./agentSessionTranscriptContext.ts"
  );
  const { AgentSessionWorkBlockSegment } = await import(
    "./AgentSessionWorkBlock.tsx"
  );

  const element = (blockItems, streaming, live) =>
    createElement(
      AgentSessionTranscriptTurnMetaProvider,
      {
        value: {
          liveTurnId: live,
          streamingItemId: streaming,
        },
      },
      createElement(AgentSessionWorkBlockSegment, {
        agentAvatarUrl: null,
        agentName: "Agent",
        agentPubkey: "pk",
        block: {
          id: `work-block:${blockItems[0].id}`,
          items: blockItems,
          timestamp: blockItems[0].timestamp,
        },
      }),
    );

  let applyState;
  const Harness = () => {
    const [state, setState] = useState({ items, liveTurnId, streamingItemId });
    applyState = setState;
    return element(state.items, state.streamingItemId, state.liveTurnId);
  };
  // The bubble presenter reads the posted message through `useQuery`. Provided
  // unconditionally so that if a rail step ever DID reach that presenter, the
  // relay test below would fail on its bubble assertion rather than on a
  // missing provider — a test must fail for the reason it claims.
  // `gcTime: 0` because React Query's default is 300000ms, and an un-collected
  // query arms a timer that long at teardown — node:test then waits it out
  // before exiting, turning a 2s suite into a 5-minute wall with no failing
  // assertion. Dormant here today (only the relay-bubble test reaches a
  // `useQuery`, and it builds its own client), but a silent five-minute hang is
  // an expensive thing to leave armed for the next rig-based test that renders
  // a bubble.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { gcTime: 0, retry: false } },
  });
  const rootRoute = createRootRoute({
    component: () =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(Harness),
      ),
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute,
  });
  await router.load();

  const view = render(createElement(RouterProvider, { router }));
  const q = (selector) => view.container.querySelector(selector);
  const qa = (selector) => [...view.container.querySelectorAll(selector)];
  const stepCount = () =>
    qa('[data-testid="transcript-work-block-step"]').length;
  return {
    ...view,
    // Re-render the SAME block id, exactly as the transcript does as work
    // streams and then finishes. `liveTurnId` is carried through so a test can
    // model the session going away under a still-running step (agent death),
    // not just the step reaching a terminal status.
    stream: (nextItems, streaming = null, live = liveTurnId) =>
      applyState({
        items: nextItems,
        liveTurnId: live,
        streamingItemId: streaming,
      }),
    summary: () => q('[data-testid="transcript-work-block-summary"]'),
    previousSteps: () =>
      q('[data-testid="transcript-work-block-previous-steps"]'),
    stepCount,
    // A finished block renders folded, so any assertion about what is ON the
    // rail has to open it first — exactly as a reader would.
    expand: async () => {
      const { act } = await import("@testing-library/react");
      await act(async () => {
        q('[data-testid="transcript-work-block-summary"]').click();
      });
    },
    // Wait for the rail to reach `expected` rows once animations have run.
    settleToStepCount: (expected) => settle(stepCount, expected),
    glyphStates: () =>
      qa("[data-step-state]").map((node) =>
        node.getAttribute("data-step-state"),
      ),
    // Which rail glyphs pulse, reported as their step states, and the same
    // narrowed to pulses that are NOT reduced-motion-safe.
    //
    // Both read `classList.contains`, which matches a whole class token, rather
    // than a `.animate-pulse` CSS selector or `className.includes` —
    // `animate-pulse` is a SUBSTRING of `motion-safe:animate-pulse`, so a
    // substring check cannot tell the guarded class from the bare one and would
    // report the reduced-motion regression as fixed.
    //
    // They return step-state STRINGS rather than the elements: a failing
    // `deepEqual` over jsdom nodes makes node:test serialize the whole DOM tree
    // and the runner dies on SIGKILL instead of printing an assertion, which
    // silently turns "the mutant was caught" into an unreadable crash.
    pulseStates: () =>
      qa("[data-step-state]")
        .filter(
          (node) =>
            node.classList.contains("motion-safe:animate-pulse") ||
            node.classList.contains("animate-pulse"),
        )
        .map((node) => node.getAttribute("data-step-state")),
    unguardedPulseStates: () =>
      qa("[data-step-state]")
        .filter((node) => node.classList.contains("animate-pulse"))
        .map((node) => node.getAttribute("data-step-state")),
    q,
    qa,
  };
}
