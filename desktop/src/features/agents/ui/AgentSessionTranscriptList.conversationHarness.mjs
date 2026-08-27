/**
 * Shared test infrastructure for the `conversation` transcript-variant suites.
 *
 * Lives in a non-test file for two reasons. The `src/**\/*.test.mjs` glob would
 * otherwise pick it up as a suite of its own, and — more importantly — the
 * ambient-formatting pins below are order-sensitive and easy to get subtly
 * wrong, so the two suites that need them must share one copy rather than
 * maintain two. See `AgentSessionTranscriptList.conversation.test.mjs` (layout
 * and lifecycle contracts) and
 * `AgentSessionTranscriptList.conversationChrome.test.mjs` (identity row and
 * code-block chrome).
 *
 * Importing this module installs the jsdom globals and registers the
 * `before`/`afterEach`/`after` hooks for the importing suite. Import it before
 * anything that reaches for React or the DOM.
 *
 * The byte-for-byte contract is the important one: `conversation` is purely
 * additive, so the `default` and `compactPreview` markup for the same
 * transcript must be byte-identical to the markup captured before the variant
 * existed. That snapshot lives in
 * AgentSessionTranscriptList.conversation.baseline.json and was produced by
 * mounting `baselineItems()` — a transcript containing every renderable item
 * kind across two sessions — on pre-change main (074561233) in a clean
 * throwaway worktree. Regenerate it only when a deliberate change to the other
 * variants is being made.
 */

import { readFileSync } from "node:fs";
import { after, afterEach, before } from "node:test";

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

export const FIXTURE_LOCALE = "en-US";
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

export const BASELINE_MARKUP = JSON.parse(
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

/**
 * Radix's `AvatarImage` renders nothing until its own preloader reports
 * `loaded` (`react-avatar/dist/index.mjs` `useImageLoadingStatus`), and jsdom
 * never fetches, so a real avatar URL would otherwise stay in `loading` forever
 * and every avatar assertion would see only the initials fallback — the exact
 * bug under test, passing vacuously. This stub reports a decoded image as soon
 * as `src` is assigned. It only affects avatars that HAVE a url: with
 * `avatarUrl: null` radix resolves to `error` and skips the preloader entirely,
 * so the byte-for-byte baseline (whose agent and author carry no avatar) is
 * untouched.
 */
class LoadedImageStub {
  constructor() {
    this._src = "";
    this.complete = false;
    this.naturalWidth = 0;
  }
  addEventListener() {}
  removeEventListener() {}
  get src() {
    return this._src;
  }
  set src(value) {
    this._src = value;
    this.complete = true;
    this.naturalWidth = 1;
  }
}
dom.window.Image = LoadedImageStub;

/**
 * Assigned in `before`. Exported as `let` so importers see the resolved values
 * through ES module live bindings rather than a snapshot taken at import time.
 */
export let act;
export let cleanup;
export let render;
let createElement;
let useState;
let createMemoryHistory;
let createRootRoute;
let createRouter;
let RouterProvider;
let AgentSessionTranscriptList;
let ThemeProvider;
let TooltipProvider;
export let resetActiveAgentTurnsStore;
export let syncAgentTurnsFromEvents;

export const AGENT = {
  agentAvatarUrl: null,
  agentName: "Test Agent",
  agentPubkey: "f".repeat(64),
};
export const AUTHOR = "a".repeat(64);
export const AUTHOR_TRUNCATED = `${AUTHOR.slice(0, 8)}…${AUTHOR.slice(-4)}`;
/**
 * What the transcript builder actually puts in a prompt item's `title`: a
 * description of the trigger that started the turn, not an identity. Real values
 * are "Prompt", "Buzz event", and title-cased event kinds like "@Mention"
 * (`agentSessionTranscriptHelpers.ts` `parsePromptText`). The author row must
 * never display this as a name.
 */
const TRIGGER_TITLE = "@Mention";
export const AUTHOR_PROFILES = {
  [AUTHOR]: {
    displayName: "Ada Lovelace",
    avatarUrl: null,
    nip05Handle: null,
    ownerPubkey: null,
  },
};
/**
 * A relay-resolved profile for the *agent*. Deliberately not a `/media/<hex>`
 * relay URL: `UserAvatar` routes those through the localhost media proxy
 * (`rewriteRelayUrl`), which would make the rendered `src` a moving target.
 */
export const AGENT_AVATAR_URL = "https://cdn.example.test/agent-profile.png";
export const AGENT_PROFILES = {
  [AGENT.agentPubkey]: {
    displayName: "Test Agent",
    avatarUrl: AGENT_AVATAR_URL,
    nip05Handle: null,
    ownerPubkey: null,
  },
};

export function items() {
  const shared = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };
  return [
    {
      ...shared,
      id: "msg:user",
      type: "message",
      renderClass: "message",
      role: "user",
      title: TRIGGER_TITLE,
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
export function baselineItems() {
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

export async function renderTranscript(variant, overrides = {}) {
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
 * Same mount, wrapped in the providers a fenced code block needs.
 *
 * `MarkdownCodeBlock` reaches for the theme (shiki highlighting) and a Radix
 * tooltip provider for its copy action, so a transcript containing a fenced
 * block throws without them. Kept as a separate helper rather than folded into
 * `renderTranscript` so the byte-for-byte fixture keeps rendering through the
 * exact tree it was captured with.
 */
export async function renderTranscriptWithCodeChrome(variant, overrides = {}) {
  const rootRoute = createRootRoute({
    component: () =>
      createElement(
        ThemeProvider,
        null,
        createElement(
          TooltipProvider,
          null,
          createElement(AgentSessionTranscriptList, {
            ...AGENT,
            emptyDescription: "nothing yet",
            items: items(),
            variant,
            ...overrides,
          }),
        ),
      ),
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute,
  });
  await router.load();
  return render(createElement(RouterProvider, { router }));
}

/** One assistant turn whose body is a fenced code block. */
export function fencedCodeItems() {
  return [
    {
      channelId: "chan-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      id: "msg:assistant",
      type: "message",
      renderClass: "message",
      role: "assistant",
      title: "Test Agent",
      text: "before\n\n```ts\nconst a = 1;\nconst b = 2;\n```\n",
      timestamp: "2026-06-14T19:00:09.000Z",
    },
  ];
}

/** One *human prompt* whose body is a fenced code block. */
export function fencedCodePromptItems() {
  return [
    {
      channelId: "chan-1",
      sessionId: "sess-1",
      turnId: "turn-1",
      id: "msg:user",
      type: "message",
      renderClass: "message",
      role: "user",
      title: TRIGGER_TITLE,
      text: "fix this\n\n```ts\nconst a = 1;\nconst b = 2;\n```\n",
      timestamp: "2026-06-14T19:00:00.000Z",
      messageId: "event-1",
      authorPubkey: AUTHOR,
    },
  ];
}

/**
 * Same mount, but the caller can swap the list props afterwards. Needed for the
 * contracts that are only visible across a rerender: a streaming thought
 * folding once the turn moves on, and a plan mutating in place.
 */
export async function renderRerenderableTranscript(
  variant,
  initialOverrides = {},
) {
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
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider.tsx"));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
});

afterEach(() => {
  cleanup?.();
  resetActiveAgentTurnsStore?.();
});
after(() => dom.window.close());
