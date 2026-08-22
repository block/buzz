/**
 * Rendering contract for the `conversation` transcript variant (focus mode).
 *
 * Mounts the shipping AgentSessionTranscriptList so the variant plumbing
 * (variant context + derived turn meta) is exercised end to end rather than
 * asserting against re-implemented render classes.
 *
 * The byte-for-byte tests at the bottom are the important ones: `conversation`
 * is purely additive, so the `default` and `compactPreview` markup for the same
 * transcript must be byte-identical to the markup captured before the variant
 * existed. That snapshot lives in
 * AgentSessionTranscriptList.conversation.baseline.json and was produced by
 * mounting `baselineItems()` — a transcript containing every renderable item
 * kind across two sessions — on pre-change main (074561233) in a clean
 * throwaway worktree. Regenerate it only when a deliberate change to the other
 * variants is being made.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, afterEach, before, test } from "node:test";

// The captured markup embeds formatted dates and times, so the fixture is only
// reproducible if every ambient formatting input is pinned. Two of them bite:
//
//  - **Zone.** `formatTranscriptTimestampTitle` formats in the ambient zone
//    ("… at 7:00:01 PM"), so a capture at UTC-7 fails against CI's UTC.
//  - **Locale.** The session-boundary divider uses a bare `toLocaleString()`
//    (`AgentSessionTranscriptChrome.tsx`), which is locale-sensitive as well as
//    zone-sensitive: "6/14/2026, 7:05:00 PM" becomes "14.6.2026, 19:05:00"
//    under de-DE. Node derives its default locale from LANG/LC_ALL, so this
//    varies by machine independently of the zone.
//
// `TZ` can be set here because `Date` reads it lazily. The locale CANNOT: node
// resolves its default locale once at startup, so assigning `process.env.LANG`
// at runtime has no effect (verified — it silently keeps the startup locale).
// Pinning it therefore means overriding the two formatting surfaces the render
// path can reach: `Intl.DateTimeFormat` when constructed with no explicit
// locale, and `Date.prototype.toLocale*`, which does NOT route through
// `Intl.DateTimeFormat` and so needs its own patch.
//
// All of this must happen before the transcript modules are imported: their
// `Intl.DateTimeFormat` instances are module-level constants that resolve zone
// and locale once, at construction.
process.env.TZ = "UTC";

const FIXTURE_LOCALE = "en-US";
const OriginalDateTimeFormat = Intl.DateTimeFormat;
// A plain function, not an arrow: the render path calls
// `new Intl.DateTimeFormat(...)`, and an arrow function is not a constructor.
// Returning a genuine instance keeps `new`, plain calls, and `instanceof` all
// working.
function LocalePinnedDateTimeFormat(locales, options) {
  return new OriginalDateTimeFormat(locales ?? FIXTURE_LOCALE, options);
}
LocalePinnedDateTimeFormat.prototype = OriginalDateTimeFormat.prototype;
LocalePinnedDateTimeFormat.supportedLocalesOf =
  OriginalDateTimeFormat.supportedLocalesOf.bind(OriginalDateTimeFormat);
Intl.DateTimeFormat = LocalePinnedDateTimeFormat;
for (const method of [
  "toLocaleString",
  "toLocaleDateString",
  "toLocaleTimeString",
]) {
  const original = Date.prototype[method];
  Date.prototype[method] = function (locales, options) {
    return original.call(this, locales ?? FIXTURE_LOCALE, options);
  };
}

import { JSDOM } from "jsdom";

const BASELINE_MARKUP = JSON.parse(
  readFileSync(
    new URL(
      "./AgentSessionTranscriptList.conversation.baseline.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

class NoopObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

Object.assign(globalThis, {
  Element: dom.window.Element,
  Event: dom.window.Event,
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  IntersectionObserver: NoopObserver,
  MutationObserver: dom.window.MutationObserver,
  Node: dom.window.Node,
  ResizeObserver: NoopObserver,
  document: dom.window.document,
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
dom.window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
dom.window.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;

let act;
let cleanup;
let render;
let createElement;
let useState;
let createMemoryHistory;
let createRootRoute;
let createRouter;
let RouterProvider;
let AgentSessionTranscriptList;
let resetActiveAgentTurnsStore;
let syncAgentTurnsFromEvents;

const AGENT = {
  agentAvatarUrl: null,
  agentName: "Test Agent",
  agentPubkey: "f".repeat(64),
};
const AUTHOR = "a".repeat(64);

function items() {
  const shared = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };
  return [
    {
      ...shared,
      id: "msg:user",
      type: "message",
      renderClass: "message",
      role: "user",
      title: "Ada",
      text: "please summarize the plan",
      timestamp: "2026-06-14T19:00:00.000Z",
      messageId: "event-1",
      authorPubkey: AUTHOR,
    },
    {
      ...shared,
      id: "thought:1",
      type: "thought",
      renderClass: "thought",
      title: "Thinking",
      text: "weighing the options",
      timestamp: "2026-06-14T19:00:02.000Z",
    },
    {
      ...shared,
      id: "plan:1",
      type: "plan",
      renderClass: "plan",
      title: "Plan",
      text: "- [x] read the transcript\n- [ ] write the summary (in progress)\n- [ ] ship it",
      timestamp: "2026-06-14T19:00:07.000Z",
    },
    {
      ...shared,
      id: "msg:assistant",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Test Agent",
      text: "Here is the summary with `code`.",
      timestamp: "2026-06-14T19:00:09.000Z",
    },
  ];
}

/**
 * Everything the legacy variants can render, in one transcript.
 *
 * The byte-for-byte contract covers `default`/`compactPreview` for EVERY item
 * kind, so the baseline input has to contain every kind rather than the happy
 * path: prompt (with prompt context and setup lifecycle so the ingress chrome
 * renders), assistant message, thought, plan, a tool item, ordinary lifecycle
 * status, error, permission — across two sessions so a session-boundary divider
 * is forced too. Where `compactPreview` deliberately suppresses a kind, that
 * absence is captured in the fixture and is therefore also protected.
 *
 * Single tool item on purpose: a run of three would collapse into a grouped
 * summary and the leaf tool row would never be captured.
 */
function baselineItems() {
  const first = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };
  const second = { channelId: "chan-1", sessionId: "sess-2", turnId: "turn-2" };
  return [
    {
      ...first,
      id: "life:setup",
      type: "lifecycle",
      renderClass: "status",
      title: "Turn started",
      text: "1 trigger",
      timestamp: "2026-06-14T19:00:00.000Z",
      acpSource: "turn_started",
    },
    {
      ...first,
      id: "meta:context",
      type: "metadata",
      renderClass: "raw-rail",
      title: "Prompt context",
      sections: [{ title: "Channel", body: "engineering" }],
      timestamp: "2026-06-14T19:00:00.500Z",
      acpSource: "session/prompt:context",
    },
    {
      ...first,
      id: "msg:user",
      type: "message",
      renderClass: "message",
      role: "user",
      title: "Ada",
      text: "please summarize the plan",
      timestamp: "2026-06-14T19:00:01.000Z",
      messageId: "event-1",
      authorPubkey: AUTHOR,
      acpSource: "session/prompt:user",
    },
    {
      ...first,
      id: "thought:1",
      type: "thought",
      renderClass: "thought",
      title: "Thinking",
      text: "weighing the options",
      timestamp: "2026-06-14T19:00:02.000Z",
    },
    {
      ...first,
      id: "plan:1",
      type: "plan",
      renderClass: "plan",
      title: "Plan",
      text: "- [x] read the transcript\n- [ ] write the summary (in progress)\n- [ ] ship it",
      timestamp: "2026-06-14T19:00:03.000Z",
    },
    {
      ...first,
      id: "tool:1",
      type: "tool",
      renderClass: "shell",
      descriptor: {
        renderClass: "shell",
        label: "Ran a command",
        preview: "cargo test",
        tone: "neutral",
        source: "shell",
      },
      title: "Ran a command",
      toolName: "shell",
      buzzToolName: null,
      status: "completed",
      args: { command: "cargo test" },
      result: "ok",
      isError: false,
      timestamp: "2026-06-14T19:00:04.000Z",
      startedAt: "2026-06-14T19:00:04.000Z",
      completedAt: "2026-06-14T19:00:05.000Z",
    },
    {
      ...first,
      id: "life:permission",
      type: "lifecycle",
      renderClass: "permission",
      title: "Permission requested",
      text: "write src/main.rs\nOptions: Allow, Deny",
      outcome: "Approved (once)",
      timestamp: "2026-06-14T19:00:06.000Z",
    },
    {
      ...first,
      id: "life:status",
      type: "lifecycle",
      renderClass: "status",
      title: "Context compacted",
      text: "",
      timestamp: "2026-06-14T19:00:07.000Z",
    },
    {
      ...first,
      id: "msg:assistant",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Test Agent",
      text: "Here is the summary with `code`.",
      timestamp: "2026-06-14T19:00:08.000Z",
    },
    {
      ...first,
      id: "life:error",
      type: "lifecycle",
      renderClass: "error",
      title: "Turn failed",
      text: "the harness exited",
      timestamp: "2026-06-14T19:00:09.000Z",
    },
    // Second session run: forces a session-boundary divider between the runs.
    {
      ...second,
      id: "msg:user2",
      type: "message",
      renderClass: "message",
      role: "user",
      title: "Ada",
      text: "next task",
      timestamp: "2026-06-14T19:05:00.000Z",
      messageId: "event-2",
      authorPubkey: AUTHOR,
      acpSource: "session/prompt:user",
    },
    {
      ...second,
      id: "msg:assistant2",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Test Agent",
      text: "on it",
      timestamp: "2026-06-14T19:05:01.000Z",
    },
  ];
}

async function renderTranscript(variant, overrides = {}) {
  const rootRoute = createRootRoute({
    component: () =>
      createElement(AgentSessionTranscriptList, {
        ...AGENT,
        emptyDescription: "nothing yet",
        items: items(),
        variant,
        ...overrides,
      }),
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute,
  });
  await router.load();
  return render(createElement(RouterProvider, { router }));
}

/**
 * Same mount, but the caller can swap the list props afterwards. Needed for the
 * contracts that are only visible across a rerender: a streaming thought
 * folding once the turn moves on, and a plan mutating in place.
 */
async function renderRerenderableTranscript(variant, initialOverrides = {}) {
  let applyProps;
  const Harness = () => {
    const [overrides, setOverrides] = useState(initialOverrides);
    applyProps = setOverrides;
    return createElement(AgentSessionTranscriptList, {
      ...AGENT,
      emptyDescription: "nothing yet",
      items: items(),
      variant,
      ...overrides,
    });
  };
  const rootRoute = createRootRoute({ component: Harness });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute,
  });
  await router.load();
  const utils = render(createElement(RouterProvider, { router }));
  return {
    ...utils,
    async setOverrides(next) {
      await act(async () => {
        applyProps(next);
      });
    },
  };
}

before(async () => {
  ({ act, cleanup, render } = await import("@testing-library/react"));
  ({ createElement, useState } = await import("react"));
  ({ createMemoryHistory, createRootRoute, createRouter, RouterProvider } =
    await import("@tanstack/react-router"));
  ({ AgentSessionTranscriptList } = await import(
    "./AgentSessionTranscriptList.tsx"
  ));
  ({ resetActiveAgentTurnsStore, syncAgentTurnsFromEvents } = await import(
    "../activeAgentTurnsStore.ts"
  ));
});

afterEach(() => {
  cleanup?.();
  resetActiveAgentTurnsStore?.();
});
after(() => dom.window.close());

test("conversation marks the transcript container and centers a reading column", async () => {
  const { container } = await renderTranscript("conversation");
  const log = container.querySelector("[data-transcript-variant]");
  assert.ok(log, "conversation should tag its container for variant styling");
  assert.equal(log.getAttribute("data-transcript-variant"), "conversation");
  assert.match(log.className, /max-w-3xl/);
  assert.match(log.className, /gap-8/);
});

test("conversation renders the prompt as a filled right-aligned bubble with an author label", async () => {
  const { container } = await renderTranscript("conversation");
  const author = container.querySelector(
    '[data-testid="transcript-user-message-author"]',
  );
  assert.ok(author, "conversation should label the prompt author");
  assert.equal(author.textContent, "Ada");

  const row = container.querySelector(
    '[data-testid="transcript-user-message"]',
  );
  assert.match(row.className, /justify-end/);
  const bubble = row.querySelector(".rounded-2xl");
  assert.match(bubble.className, /bg-muted\/60/);
  // Focus mode shows the whole prompt rather than clamping it.
  assert.doesNotMatch(bubble.className, /max-h-36/);
});

test("conversation renders agent messages as unboxed prose at full fidelity", async () => {
  const { container } = await renderTranscript("conversation");
  const message = container.querySelector(
    '[data-testid="transcript-assistant-message"]',
  );
  assert.ok(message, "assistant message should render");
  assert.doesNotMatch(message.innerHTML, /rounded-2xl/);
  // The shared markdown renderer is reused, so inline code still renders as code.
  assert.ok(message.querySelector("code"), "markdown/code fidelity preserved");
});

test("conversation collapses a finished thought into a Thought for Ns disclosure", async () => {
  const { container } = await renderTranscript("conversation");
  const disclosure = container.querySelector(
    '[data-testid="transcript-thought-item"]',
  );
  assert.ok(disclosure, "thought should render");
  assert.equal(disclosure.tagName, "DETAILS");
  assert.equal(disclosure.open, false, "finished thoughts start collapsed");
  // thought at :02, next turn item (plan) at :07 → 5s.
  assert.equal(
    disclosure
      .querySelector('[data-testid="transcript-thought-disclosure"]')
      .textContent.trim(),
    "Thought for 5s",
  );
});

test("conversation auto-opens the thought disclosure while it is streaming", async () => {
  // A live turn on this channel makes the trailing item the streaming one, which
  // is exactly the condition the disclosure auto-opens for.
  syncAgentTurnsFromEvents(AGENT.agentPubkey, [
    {
      seq: 1,
      timestamp: "2026-06-14T19:00:02.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "chan-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: null,
    },
  ]);
  const streamingThought = items().slice(0, 2);
  const { container } = await renderTranscript("conversation", {
    channelId: "chan-1",
    items: streamingThought,
  });

  const disclosure = container.querySelector(
    '[data-testid="transcript-thought-item"]',
  );
  assert.equal(disclosure.open, true, "streaming thought should be open");
  const summary = disclosure.querySelector(
    '[data-testid="transcript-thought-disclosure"]',
  );
  // Shimmer paints a visual-only aria-hidden duplicate of the label, so match a
  // prefix rather than the whole text node.
  assert.match(summary.textContent, /^Thinking…/);
  assert.doesNotMatch(summary.textContent, /Thought for/);
});

test("conversation folds the thought when the turn moves on, even after the browser echoes the auto-open toggle", async () => {
  // Regression guard for the programmatic-toggle echo trap: a real browser fires
  // `toggle` when React flips `open`, so the auto-open for a streaming thought
  // arrives back at the component as if the reader had clicked. JSDOM does not
  // fire that event itself, so the test injects it. If the handler records it as
  // a reader choice, the disclosure stays pinned open forever and reasoning
  // never recedes once the agent acts.
  syncAgentTurnsFromEvents(AGENT.agentPubkey, [
    {
      seq: 1,
      timestamp: "2026-06-14T19:00:02.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "chan-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: null,
    },
  ]);
  const streaming = { channelId: "chan-1", items: items().slice(0, 2) };
  const { container, setOverrides } = await renderRerenderableTranscript(
    "conversation",
    streaming,
  );

  const disclosure = container.querySelector(
    '[data-testid="transcript-thought-item"]',
  );
  assert.equal(disclosure.open, true, "streaming thought should auto-open");

  // The browser echo: `open` already agrees with what React rendered, and the
  // event follows rather than causes that state.
  await act(async () => {
    disclosure.open = true;
    disclosure.dispatchEvent(new dom.window.Event("toggle"));
  });
  assert.equal(
    disclosure.open,
    true,
    "the echo must not disturb the open tail",
  );

  // The turn produces its next item, so the thought is no longer the streaming
  // tail and should recede.
  await setOverrides({ ...streaming, items: items().slice(0, 3) });

  const settled = container.querySelector(
    '[data-testid="transcript-thought-item"]',
  );
  assert.equal(
    settled.open,
    false,
    "a thought the agent has acted on should fold again",
  );
  assert.match(
    settled
      .querySelector('[data-testid="transcript-thought-disclosure"]')
      .textContent.trim(),
    /^Thought for 5s/,
  );
});

test("conversation keeps a reader-opened thought open after the turn moves on", async () => {
  // The other half of the guard: a toggle that DISAGREES with the rendered state
  // is a genuine reader choice and must win over the stream transition.
  syncAgentTurnsFromEvents(AGENT.agentPubkey, [
    {
      seq: 1,
      timestamp: "2026-06-14T19:00:02.000Z",
      kind: "turn_started",
      agentIndex: 0,
      channelId: "chan-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      payload: null,
    },
  ]);
  // Three items so the thought is not the streaming tail: it renders collapsed.
  const settledItems = { channelId: "chan-1", items: items().slice(0, 3) };
  const { container, setOverrides } = await renderRerenderableTranscript(
    "conversation",
    settledItems,
  );

  const disclosure = container.querySelector(
    '[data-testid="transcript-thought-item"]',
  );
  assert.equal(disclosure.open, false, "a settled thought starts collapsed");

  await act(async () => {
    disclosure.open = true;
    disclosure.dispatchEvent(new dom.window.Event("toggle"));
  });

  await setOverrides({ ...settledItems, items: items() });

  assert.equal(
    container.querySelector('[data-testid="transcript-thought-item"]').open,
    true,
    "the reader's choice should survive later transcript items",
  );
});

test("conversation renders the plan as a checklist card with in-place progress", async () => {
  const { container } = await renderTranscript("conversation");
  const card = container.querySelector('[data-testid="transcript-plan-item"]');
  assert.ok(card, "plan should render");
  assert.equal(card.getAttribute("data-variant"), "conversation-plan-card");
  assert.equal(
    card.querySelector('[data-testid="transcript-plan-progress"]').textContent,
    "1/3 complete",
  );
  const statuses = [...card.querySelectorAll("[data-plan-entry-status]")].map(
    (entry) => entry.getAttribute("data-plan-entry-status"),
  );
  assert.deepEqual(statuses, ["completed", "in_progress", "pending"]);
});

test("conversation updates the same plan card in place as entries advance", async () => {
  // The transcript reducer mutates the plan item in place (same item id), so the
  // card must be re-rendered rather than replaced: progress copy changes while
  // the same DOM node is retained. A new node here would mean the reader's
  // scroll position and any card-level state get thrown away on every update.
  const { container, setOverrides } =
    await renderRerenderableTranscript("conversation");
  const before = container.querySelector(
    '[data-testid="transcript-plan-item"]',
  );
  assert.equal(
    before.querySelector('[data-testid="transcript-plan-progress"]')
      .textContent,
    "1/3 complete",
  );

  const advanced = items().map((item) =>
    item.id === "plan:1"
      ? {
          ...item,
          text: "- [x] read the transcript\n- [x] write the summary\n- [ ] ship it (in progress)",
        }
      : item,
  );
  await setOverrides({ items: advanced });

  const after = container.querySelector('[data-testid="transcript-plan-item"]');
  assert.equal(after, before, "the same plan card node should be reused");
  assert.equal(
    after.querySelector('[data-testid="transcript-plan-progress"]').textContent,
    "2/3 complete",
  );
  assert.deepEqual(
    [...after.querySelectorAll("[data-plan-entry-status]")].map((entry) =>
      entry.getAttribute("data-plan-entry-status"),
    ),
    ["completed", "completed", "in_progress"],
  );
});

test("conversation quiets session boundaries and status rows into centered dividers", async () => {
  const shared = { channelId: "chan-1", sessionId: "sess-2", turnId: "turn-2" };
  const { container } = await renderTranscript("conversation", {
    items: [
      ...items(),
      {
        ...shared,
        id: "msg:user2",
        type: "message",
        renderClass: "message",
        role: "user",
        title: "Ada",
        text: "next task",
        timestamp: "2026-06-14T19:05:01.000Z",
        messageId: "event-2",
        authorPubkey: AUTHOR,
      },
      {
        ...shared,
        id: "life:status",
        type: "lifecycle",
        renderClass: "status",
        title: "Context compacted",
        text: "",
        timestamp: "2026-06-14T19:05:02.000Z",
      },
    ],
  });

  // A second session id draws the boundary rule between the two runs.
  const boundary = container.querySelector(
    '[data-testid="session-boundary-divider"]',
  );
  assert.ok(boundary, "a second session should draw a boundary");
  assert.equal(boundary.getAttribute("data-variant"), "conversation-divider");

  const status = container.querySelector(
    '[data-testid="transcript-lifecycle-item"]',
  );
  assert.ok(status, "status lifecycle row should render");
  assert.equal(status.getAttribute("data-variant"), "conversation-divider");
  assert.match(status.textContent, /Context compacted/);
});

test("conversation keeps errors and permission gates loud", async () => {
  const shared = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };
  const { container } = await renderTranscript("conversation", {
    items: [
      ...items(),
      {
        ...shared,
        id: "life:permission",
        type: "lifecycle",
        renderClass: "permission",
        title: "Permission requested",
        text: "write src/main.rs\nOptions: Allow, Deny",
        outcome: "Approved (once)",
        timestamp: "2026-06-14T19:00:11.000Z",
      },
      {
        ...shared,
        id: "life:error",
        type: "lifecycle",
        renderClass: "error",
        title: "Turn failed",
        text: "the harness exited",
        timestamp: "2026-06-14T19:00:12.000Z",
      },
    ],
  });

  const permission = container.querySelector(
    '[data-testid="transcript-permission-item"]',
  );
  assert.ok(permission, "permission item should render");
  assert.match(permission.className, /amber/);
  assert.equal(
    permission.getAttribute("data-variant"),
    null,
    "permission gates never route through the quiet divider",
  );
  assert.equal(
    permission
      .querySelector('[data-testid="transcript-permission-outcome"]')
      .textContent.trim(),
    "Approved (once)",
  );

  // The error shares the lifecycle testid, so pick it out by its copy.
  const error = [
    ...container.querySelectorAll('[data-testid="transcript-lifecycle-item"]'),
  ].find((node) => node.textContent.includes("Turn failed"));
  assert.ok(error, "error lifecycle item should still render");
  assert.match(error.className, /destructive/);
  assert.equal(
    error.getAttribute("data-variant"),
    null,
    "errors never route through the quiet divider",
  );
});

test("default and compactPreview markup is unchanged by the conversation variant", async () => {
  for (const variant of ["default", "compactPreview"]) {
    const { container } = await renderTranscript(variant, {
      items: baselineItems(),
    });
    assert.equal(
      container.innerHTML,
      BASELINE_MARKUP[variant],
      `${variant} transcript markup drifted`,
    );
    cleanup();
  }
});

test("the byte-for-byte baseline actually exercises every renderable item kind", async () => {
  // The fixture embeds formatted dates and times, so a zone or locale other than
  // the one it was captured in fails the comparison above for a reason that has
  // nothing to do with markup. Fail loudly and specifically instead.
  assert.equal(
    new Intl.DateTimeFormat().resolvedOptions().timeZone,
    "UTC",
    "the baseline fixture is captured in UTC — see the TZ pin at the top of this file",
  );
  assert.equal(
    new Intl.DateTimeFormat().resolvedOptions().locale,
    FIXTURE_LOCALE,
    `the baseline fixture is captured in ${FIXTURE_LOCALE} — see the locale pin at the top of this file`,
  );
  assert.equal(
    new Date("2026-06-14T19:05:00.000Z").toLocaleString(),
    "6/14/2026, 7:05:00 PM",
    "Date.prototype.toLocaleString must be locale-pinned too — the session-boundary divider uses it",
  );
  // Guards the fixture itself: a baseline that silently stopped covering a kind
  // would keep passing the comparison above while protecting nothing. Asserted
  // against `default`, which renders every kind.
  const { container } = await renderTranscript("default", {
    items: baselineItems(),
  });
  const present = (selector) => container.querySelector(selector) !== null;
  for (const [kind, selector] of [
    ["prompt", '[data-testid="transcript-user-message"]'],
    ["prompt setup", '[data-testid="transcript-turn-setup"]'],
    ["assistant message", '[data-testid="transcript-assistant-message"]'],
    ["thought", '[data-testid="transcript-thought-item"]'],
    ["plan", '[data-testid="transcript-plan-item"]'],
    ["tool", '[data-testid="transcript-tool-item"]'],
    ["permission", '[data-testid="transcript-permission-item"]'],
    ["lifecycle status/error", '[data-testid="transcript-lifecycle-item"]'],
    ["session boundary", '[data-testid="session-boundary-divider"]'],
  ]) {
    assert.ok(present(selector), `baseline must cover ${kind}`);
  }
  // Status and error share the lifecycle testid, so check both are really there.
  const lifecycleText = [
    ...container.querySelectorAll('[data-testid="transcript-lifecycle-item"]'),
  ]
    .map((node) => node.textContent)
    .join("\n");
  assert.match(lifecycleText, /Context compacted/);
  assert.match(lifecycleText, /Turn failed/);
});
