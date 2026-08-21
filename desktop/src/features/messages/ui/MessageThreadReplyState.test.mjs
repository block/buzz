/**
 * Mount regressions for the thread reply region terminal surface, wired
 * through the panel-owned ThreadRepliesTerminalCard.
 *
 * Bug this pins: a terminal thread-replies fetch error used to fall through to
 * the "No replies in this branch yet" empty card, silently presenting a broken
 * load as an authoritative empty branch with no recovery. The fix maps a
 * terminal error surface to the retry card and NEVER the empty card, and routes
 * the Retry button back to the query's refetch. selectThreadRepliesSurface
 * (unit tested in timelineSnapshot.test.mjs) picks the surface; this file
 * proves the panel's consumer renders the right card and fires retry.
 *
 * Mutation proof: ThreadRepliesTerminalCard is exported from
 * MessageThreadPanel.tsx, the production consumer of
 * threadRepliesError/onRetryThreadReplies. Reverting the panel to its pre-fix
 * state removes this export, so the import throws and every test here fails.
 * Mounting the full panel is infeasible in node:test (its Tiptap composer /
 * React Query stack is unavailable, see MessageComposerAutoSend.test.mjs), so
 * the terminal-surface consumer is extracted to this cheap-to-mount component.
 *
 * Wiring guard: mounting the exported card proves the surface→card mapping,
 * but not that the panel still ROUTES its real repliesSurface / retry callback
 * through it — the panel could drop the card and render an unconditional empty
 * state, restoring the false-empty bug while these mount tests stay green. The
 * final test is a structural source tripwire that fails if the panel's terminal
 * branch stops rendering ThreadRepliesTerminalCard fed by both
 * surface={repliesSurface} and onRetry={onRetryThreadReplies}. Full panel mount
 * can't observe this call site, so the source is the binding.
 *
 * CI surface: pnpm test (node:test with @testing-library/react over JSDOM).
 */

import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { after, afterEach, before, test } from "node:test";
import { fileURLToPath } from "node:url";

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

async function renderCard(props) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ThreadRepliesTerminalCard } = await import(
    "./MessageThreadPanel.tsx"
  );
  return render(createElement(ThreadRepliesTerminalCard, props));
}

test("terminal error renders the retry card, never the empty card", async () => {
  const { screen } = await import("@testing-library/react");
  await renderCard({ surface: "error", onRetry: () => {} });

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
  await renderCard({
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
  await renderCard({ surface: "empty" });

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
  const { container } = await renderCard({ surface: "pending" });

  assert.equal(
    container.textContent,
    "",
    "the pending surface must render nothing while rows stream in",
  );
});

test("panel routes its real reply surface + retry through the terminal card", () => {
  // Structural tripwire: the mount tests above prove the card maps surfaces to
  // the right subcards, but they never observe the panel's call site. This
  // guard binds that call site — if the terminal branch stops rendering
  // <ThreadRepliesTerminalCard> fed by both the live repliesSurface and the
  // retry callback (e.g. reverted to an unconditional empty card), the panel
  // false-empty bug is back with these tests still green. Full panel mount
  // can't reach this JSX (Tiptap composer / React Query stack), so the source
  // is the only place to bind it.
  const panelPath = path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "MessageThreadPanel.tsx",
  );
  const source = fs.readFileSync(panelPath, "utf8");

  // Isolate the JSX open tag (not the `function ThreadRepliesTerminalCard`
  // definition) and its prop list.
  const openTag = source.match(/<ThreadRepliesTerminalCard\b[^>]*\/>/s);
  assert.ok(
    openTag,
    "the panel's terminal reply branch must render <ThreadRepliesTerminalCard />",
  );
  const props = openTag[0].replace(/\s+/g, " ");
  assert.match(
    props,
    /surface=\{repliesSurface\}/,
    "the terminal card must be fed the live repliesSurface",
  );
  assert.match(
    props,
    /onRetry=\{onRetryThreadReplies\}/,
    "the terminal card must be fed the panel's retry callback",
  );
});
