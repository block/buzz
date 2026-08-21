/**
 * Mount regressions for the thread reply region, wired through the
 * panel-owned ThreadReplyRegion dispatcher.
 *
 * Bug this pins: a terminal thread-replies fetch error used to fall through to
 * the "No replies in this branch yet" empty card, silently presenting a broken
 * load as an authoritative empty branch with no recovery. The fix maps a
 * terminal error surface to the retry card and NEVER the empty card, and routes
 * the Retry button back to the query's refetch. selectThreadRepliesSurface
 * (unit tested in timelineSnapshot.test.mjs) picks the surface; this file mounts
 * ThreadReplyRegion — the component that OWNS the surface→content branching —
 * and drives every surface through it.
 *
 * Why this component, not the panel: MessageThreadPanel delegates its whole
 * reply region to ThreadReplyRegion, passing the live repliesSurface, its retry
 * callback, and render callbacks for the two heavy (skeleton/list) branches. The
 * error≠empty decision lives inside ThreadReplyRegion, so mounting it exercises
 * the real production branching — an unwire in the panel drops the region
 * entirely rather than leaving a silently-dead card. Mounting the full panel is
 * infeasible in node:test (its Tiptap composer / React Query stack is
 * unavailable, see MessageComposerAutoSend.test.mjs); the render callbacks keep
 * that heavy construction in the panel and out of this cheap mount.
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
      renderSkeleton: () => createElement("div", null, SKELETON_MARK),
      renderList: () => createElement("div", null, LIST_MARK),
      ...props,
    }),
  );
}

test("terminal error renders the retry card, never the empty card", async () => {
  const { screen } = await import("@testing-library/react");
  await renderRegion({ surface: "error", onRetry: () => {} });

  assert.ok(
    screen.getByTestId("message-thread-replies-error"),
    "a terminal error must render the error card",
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
    surface: "error",
    onRetry: () => {
      retryCount += 1;
    },
  });

  fireEvent.click(screen.getByTestId("message-thread-replies-retry"));

  assert.equal(retryCount, 1, "clicking Retry must call the refetch callback");
});

test("genuine empty surface renders the empty card, not the error card", async () => {
  const { screen } = await import("@testing-library/react");
  await renderRegion({ surface: "empty" });

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
  const { container } = await renderRegion({ surface: "pending" });

  assert.equal(
    container.textContent,
    "",
    "the pending surface must render nothing while rows stream in",
  );
});

test("skeleton surface renders the panel's skeleton branch", async () => {
  const { container } = await renderRegion({ surface: "skeleton" });

  assert.equal(
    container.textContent,
    SKELETON_MARK,
    "the skeleton surface must render the panel's skeleton branch",
  );
});

test("list surface renders the panel's list branch", async () => {
  const { container } = await renderRegion({ surface: "list" });

  assert.equal(
    container.textContent,
    LIST_MARK,
    "the list surface must render the panel's list branch",
  );
});
