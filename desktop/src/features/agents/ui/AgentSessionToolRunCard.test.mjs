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
    Element: dom.window.Element,
    Node: dom.window.Node,
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

const START = "2026-06-18T00:00:00.000Z";

function step(id, overrides = {}) {
  return {
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
    turnId: "turn-1",
    sessionId: "sess-1",
    channelId: "chan-1",
    ...overrides,
  };
}

async function renderCard(items) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { AgentSessionToolRunCard } = await import(
    "./AgentSessionToolRunCard.tsx"
  );

  const element = (runItems) =>
    createElement(AgentSessionToolRunCard, {
      agentAvatarUrl: null,
      agentName: "Agent",
      agentPubkey: "pk",
      run: {
        id: `tool-run:${runItems[0].id}`,
        items: runItems,
        timestamp: runItems[0].timestamp,
      },
    });

  const view = render(element(items));
  return {
    ...view,
    card: () => view.container.querySelector("details"),
    // Re-render the SAME card id with more/updated steps, exactly as the
    // transcript does while a run streams.
    stream: (nextItems) => view.rerender(element(nextItems)),
  };
}

/**
 * Renders a run through the variant boundary itself (`AgentSessionToolRunSegment`)
 * under the compact-preview variant, so the test proves which presentation the
 * boundary actually picks rather than assuming it.
 */
async function renderCompactPreviewRun(items) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { AgentSessionTranscriptVariantProvider } = await import(
    "./agentSessionTranscriptContext.ts"
  );
  const { AgentSessionToolRunSegment } = await import(
    "./AgentSessionToolRunCard.tsx"
  );

  const view = render(
    createElement(
      AgentSessionTranscriptVariantProvider,
      { value: "compactPreview" },
      createElement(AgentSessionToolRunSegment, {
        agentAvatarUrl: null,
        agentName: "Agent",
        agentPubkey: "pk",
        run: {
          id: `tool-run:${items[0].id}`,
          items,
          timestamp: items[0].timestamp,
        },
      }),
    ),
  );
  return { ...view, row: () => view.container.querySelector("details") };
}

// ── Disclosure ───────────────────────────────────────────────────────────────

test("a live run is expanded so the reader watches work happen", async () => {
  const { card } = await renderCard([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(card().open, true);
});

// The regression this guards: `<details>` fires `toggle` for programmatic
// `open` changes too. If the card's own auto-expand were mistaken for a reader
// choice, the completed run would stay pinned open forever.
test("a run that completes collapses itself without the reader touching it", async () => {
  const { card, stream } = await renderCard([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(card().open, true);

  stream([step("a"), step("b")]);
  assert.equal(card().open, false);
});

// Real browsers fire `toggle` for programmatic `open` changes too, so the
// card's own auto-expand arrives back as an event that AGREES with the state we
// just rendered. jsdom does not emit that echo, so it is injected here: without
// the guard the echo is recorded as a reader choice and pins the card open, and
// the completed run never collapses.
test("a browser toggle echo of the card's own auto-expand is not a reader choice", async () => {
  const { act, fireEvent } = await import("@testing-library/react");
  const { card, stream } = await renderCard([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(card().open, true);

  // The echo: open state already matches what we rendered.
  await act(async () => {
    fireEvent(card(), new dom.window.Event("toggle"));
  });

  stream([step("a"), step("b")]);
  assert.equal(card().open, false);
});

test("a completed run that contains a failure stays open", async () => {
  const { card, stream } = await renderCard([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);

  stream([step("a"), step("b", { isError: true, status: "failed" })]);
  assert.equal(card().open, true);
});

test("a settled clean run renders collapsed from the start", async () => {
  const { card } = await renderCard([step("a"), step("b")]);
  assert.equal(card().open, false);
});

test("the reader's collapse survives later streaming updates", async () => {
  const { card, stream } = await renderCard([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(card().open, true);

  const { act, fireEvent } = await import("@testing-library/react");
  // Reader collapses a live run: emulate the click plus the browser's own
  // open-state mutation, which is what actually fires `toggle`.
  await act(async () => {
    card().open = false;
    fireEvent(card(), new dom.window.Event("toggle"));
  });
  assert.equal(card().open, false);

  // A new step arrives; the reader's choice still wins over "live means open".
  stream([
    step("a"),
    step("b"),
    step("c", { status: "executing", completedAt: null }),
  ]);
  assert.equal(card().open, false);
});

test("the reader can open a failed run's card and keep it open", async () => {
  const { card, stream } = await renderCard([
    step("a", { isError: true, status: "failed" }),
    step("b"),
  ]);
  assert.equal(card().open, true);

  const { act, fireEvent } = await import("@testing-library/react");
  await act(async () => {
    card().open = false;
    fireEvent(card(), new dom.window.Event("toggle"));
  });
  assert.equal(card().open, false);

  stream([step("a", { isError: true, status: "failed" }), step("b")]);
  assert.equal(card().open, false);
});

// ── Header ───────────────────────────────────────────────────────────────────

test("the header reads as an outcome once settled and names the failing count", async () => {
  const { container, getByText } = await renderCard([
    step("a"),
    step("b", { isError: true, status: "failed" }),
  ]);

  assert.equal(
    container.querySelector("[data-run-phase]").dataset.runPhase,
    "error",
  );
  // Aggregate glyph is announced, not just drawn.
  getByText("1 step failed");
});

test("the header reads as active with the step position while live", async () => {
  const { container } = await renderCard([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);

  // Scope to the card's own summary: the executing step row inside the body
  // announces its own status too.
  const header = container.querySelector("details > summary");
  assert.match(header.textContent, /Running/);
  assert.match(header.textContent, /step 2/);
});

test("a clean settled run announces done", async () => {
  const { container, getByText } = await renderCard([step("a"), step("b")]);
  assert.equal(
    container.querySelector("[data-run-phase]").dataset.runPhase,
    "done",
  );
  getByText("Done");
});

// ── Body ─────────────────────────────────────────────────────────────────────

test("the body renders one row per step and highlights the failing one", async () => {
  const { container } = await renderCard([
    step("a"),
    step("b", { isError: true, status: "failed" }),
    step("c"),
  ]);

  const rows = container.querySelectorAll(
    '[data-testid="transcript-tool-run-step"]',
  );
  assert.equal(rows.length, 3);
  assert.deepEqual(
    [...rows].map((row) => row.dataset.stepFailed),
    [undefined, "true", undefined],
  );
});

test("the card carries the run id so streaming steps never remount it", async () => {
  const { container } = await renderCard([step("a"), step("b")]);
  assert.equal(
    container.querySelector("[data-tool-run-id]").dataset.toolRunId,
    "tool-run:a",
  );
});

// ── Compact preview ──────────────────────────────────────────────────────────
//
// The compact activity preview is a passive thumbnail, so a run must read there
// exactly as it did before chain cards existed: the generic `Ran N tool calls`
// sentence, plain and self-managed. These tests lock that contract — the chain
// card's chrome and policy leaking into the preview is a regression.

test("compact preview renders a run as the plain legacy summary row", async () => {
  const { container, queryByTestId, row } = await renderCompactPreviewRun([
    step("a"),
    step("b"),
  ]);

  // The full chain card is not what the boundary picked.
  assert.equal(queryByTestId("transcript-tool-run-card"), null);
  assert.ok(queryByTestId("transcript-tool-run-compact-row"));
  // Scoped to the run's own summary, because the step rows in the body carry
  // their own "Ran <command>" labels. The count animates through
  // AnimatedCount, so the sentence is asserted as text rather than as one node.
  const summary = container.querySelector("details > summary");
  // The generic sentence, not a derived verb/object headline ("Ran 2 commands").
  assert.match(summary.textContent, /^Ran/);
  assert.match(summary.textContent, /2 tool calls$/);
  assert.equal(row().open, false);
});

test("compact preview draws no aggregate status glyph or timing", async () => {
  const { container, queryByText } = await renderCompactPreviewRun([
    step("a"),
    step("b"),
  ]);

  // The glyphs announce themselves for screen readers, so their absence is
  // assertable rather than a matter of inspecting class names.
  assert.equal(queryByText("Done"), null);
  assert.equal(queryByText("Running"), null);
  assert.equal(queryByText("1 step failed"), null);
  // No run-level phase/timing chrome at all.
  assert.equal(container.querySelector("[data-run-phase]"), null);
  assert.equal(
    container.querySelector("[data-testid='transcript-row-timestamp']"),
    null,
  );
});

// No auto-open while live and no auto-collapse on settle: disclosure in the
// preview is the `<details>` element's own business.
test("compact preview applies no auto-open or auto-collapse policy", async () => {
  const live = await renderCompactPreviewRun([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(live.row().open, false);

  const failed = await renderCompactPreviewRun([
    step("a"),
    step("b", { isError: true, status: "failed" }),
  ]);
  assert.equal(failed.row().open, false);
});

test("compact preview still expands to one row per step", async () => {
  const { container } = await renderCompactPreviewRun([
    step("a"),
    step("b"),
    step("c"),
  ]);
  assert.equal(
    container.querySelectorAll('[data-testid="transcript-tool-run-step"]')
      .length,
    3,
  );
});

test("the default variant gets the full chain card, not the plain row", async () => {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { AgentSessionTranscriptVariantProvider } = await import(
    "./agentSessionTranscriptContext.ts"
  );
  const { AgentSessionToolRunSegment } = await import(
    "./AgentSessionToolRunCard.tsx"
  );

  const items = [step("a"), step("b")];
  const { queryByTestId } = render(
    createElement(
      AgentSessionTranscriptVariantProvider,
      { value: "default" },
      createElement(AgentSessionToolRunSegment, {
        agentAvatarUrl: null,
        agentName: "Agent",
        agentPubkey: "pk",
        run: { id: "tool-run:a", items, timestamp: items[0].timestamp },
      }),
    ),
  );

  assert.ok(queryByTestId("transcript-tool-run-card"));
  assert.equal(queryByTestId("transcript-tool-run-compact-row"), null);
});

// ── Streaming cost ───────────────────────────────────────────────────────────

/**
 * A run is re-rendered on every append while it streams (and on every
 * live-clock tick). Unchanged steps must not re-render with it: each step's
 * presenter rebuilds compact tool summaries, parses diffs, and renders
 * markdown/images, so an unmemoized step row makes a long run cost O(n) of that
 * work per appended step.
 *
 * Counted at the presenter boundary — `TranscriptActivityItem` looks its
 * presenter up in `ACTIVITY_RENDER_CLASS_PRESENTERS` on every render, so
 * swapping in a counting presenter observes exactly the work a step row
 * triggers, without reaching into React internals.
 */
async function countStepRenders(initialItems, nextItems) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ACTIVITY_RENDER_CLASS_PRESENTERS } = await import(
    "./activityRenderClasses/TranscriptActivityItem.tsx"
  );
  const { AgentSessionToolRunCard } = await import(
    "./AgentSessionToolRunCard.tsx"
  );

  const renders = [];
  const original = ACTIVITY_RENDER_CLASS_PRESENTERS.shell;
  ACTIVITY_RENDER_CLASS_PRESENTERS.shell = function CountingPresenter(props) {
    renders.push(props.item.id);
    return createElement("div", null, props.item.id);
  };

  try {
    const element = (items) =>
      createElement(AgentSessionToolRunCard, {
        agentAvatarUrl: null,
        agentName: "Agent",
        agentPubkey: "pk",
        run: { id: "tool-run:a", items, timestamp: items[0].timestamp },
      });

    const view = render(element(initialItems));
    renders.length = 0;
    view.rerender(element(nextItems));
    return renders;
  } finally {
    ACTIVITY_RENDER_CLASS_PRESENTERS.shell = original;
  }
}

test("appending a step does not re-render the steps already in the run", async () => {
  // Five settled steps, then a sixth arrives. The five are the SAME objects
  // across both renders, as the transcript store replaces items rather than
  // mutating them.
  const settled = ["a", "b", "c", "d", "e"].map((id) => step(id));
  const appended = [
    ...settled,
    step("f", { status: "executing", completedAt: null }),
  ];

  const rendered = await countStepRenders(settled, appended);

  // Only the newly appended step renders; the five unchanged ones are skipped.
  assert.deepEqual(rendered, ["f"]);
});

test("a step that actually changed does re-render", async () => {
  // Guards the memo from being too aggressive: an executing step settling is a
  // new object for that id, and it must re-render to drop its spinner.
  const a = step("a");
  const executing = step("b", { status: "executing", completedAt: null });
  const settled = step("b");

  const rendered = await countStepRenders([a, executing], [a, settled]);

  assert.deepEqual(rendered, ["b"]);
});
