/**
 * Mount regressions for the thread reply region, wired through the
 * panel-owned ThreadReplyRegion dispatcher.
 *
 * Bug this pins: a terminal thread-replies fetch error used to fall through to
 * the "No replies in this branch yet" empty card, silently presenting a broken
 * load as an authoritative empty branch with no recovery. The fix maps a
 * terminal error to the retry card and NEVER the empty card, and routes the
 * Retry button back to the query's refetch.
 *
 * Why this component, and why raw inputs: ThreadReplyRegion now owns BOTH the
 * surface selection (selectThreadRepliesSurface, also unit-tested against its
 * 8-case matrix in timelineSnapshot.test.mjs) AND the surface→content dispatch.
 * MessageThreadPanel passes only its raw query/render state — the pending/error
 * flags and the deferred vs. live reply counts — so there is no precomputed
 * `surface` prop at the panel boundary to statically mis-set. This file mounts
 * ThreadReplyRegion and drives the real raw-state→surface→content mapping, so a
 * false-empty regression cannot land at either the selection or the dispatch
 * with these tests green. Mounting the full panel is infeasible in node:test
 * (its Tiptap composer / React Query stack is unavailable, see
 * MessageComposerAutoSend.test.mjs); the render callbacks keep that heavy
 * construction in the panel and out of this cheap mount.
 *
 * CI surface: pnpm test (node:test with @testing-library/react over JSDOM).
 */

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
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

// Sentinels for the two heavy branches the panel owns. If ThreadReplyRegion
// ever routes error/empty/pending through a render callback, these appear where
// a card is expected and the assertions catch it.
const SKELETON_MARK = "SKELETON_BRANCH_MARKER";
const LIST_MARK = "LIST_BRANCH_MARKER";

async function renderRegion(props) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ThreadReplyRegion } = await import("./MessageThreadReplyState.tsx");
  return render(
    createElement(ThreadReplyRegion, {
      isPending: false,
      isError: false,
      deferredCount: 0,
      liveCount: 0,
      renderSkeleton: () => createElement("div", null, SKELETON_MARK),
      renderList: () => createElement("div", null, LIST_MARK),
      ...props,
    }),
  );
}

test("terminal error renders the retry card, never the empty card", async () => {
  const { screen } = await import("@testing-library/react");
  // Raw terminal-failure state: not pending, load errored, nothing to show.
  await renderRegion({ isError: true, onRetry: () => {} });

  const card = screen.getByTestId("message-thread-replies-error");
  assert.ok(card, "a terminal error must render the error card");
  // The card appears asynchronously (after the query/retry lifecycle), so it
  // must be an alert live region or a screen-reader user never hears it.
  assert.equal(
    card.getAttribute("role"),
    "alert",
    "the async error card must be an alert live region for assistive tech",
  );
  assert.equal(
    document.body.textContent.includes("No replies in this branch yet"),
    false,
    "a terminal error must NEVER render the empty state",
  );
});

test("Retry button invokes the supplied refetch callback", async () => {
  const { fireEvent, screen } = await import("@testing-library/react");
  let retryCount = 0;
  await renderRegion({
    isError: true,
    onRetry: () => {
      retryCount += 1;
    },
  });

  fireEvent.click(screen.getByTestId("message-thread-replies-retry"));

  assert.equal(retryCount, 1, "clicking Retry must call the refetch callback");
});

test("genuine empty surface renders the empty card, not the error card", async () => {
  const { screen } = await import("@testing-library/react");
  // Load succeeded (no error), branch is genuinely empty.
  await renderRegion({});

  assert.ok(
    document.body.textContent.includes("No replies in this branch yet"),
    "a genuine empty branch must render the empty card",
  );
  assert.equal(
    screen.queryByTestId("message-thread-replies-error"),
    null,
    "a genuine empty branch must NOT render the error card",
  );
});

test("pending surface paints nothing", async () => {
  // Deferred snapshot is empty but the live list has content: rows are
  // streaming in on the deferred commit, so paint nothing yet.
  const { container } = await renderRegion({ deferredCount: 0, liveCount: 1 });

  assert.equal(
    container.textContent,
    "",
    "the pending surface must render nothing while rows stream in",
  );
});

test("skeleton surface renders the panel's skeleton branch", async () => {
  const { container } = await renderRegion({ isPending: true });

  assert.equal(
    container.textContent,
    SKELETON_MARK,
    "the skeleton surface must render the panel's skeleton branch",
  );
});

test("list surface renders the panel's list branch", async () => {
  const { container } = await renderRegion({ deferredCount: 1, liveCount: 1 });

  assert.equal(
    container.textContent,
    LIST_MARK,
    "the list surface must render the panel's list branch",
  );
});
