import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    KeyboardEvent: dom.window.KeyboardEvent,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    cancelAnimationFrame: dom.window.cancelAnimationFrame,
    document: dom.window.document,
    requestAnimationFrame: dom.window.requestAnimationFrame,
    window: dom.window,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia ??= () => ({
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

/**
 * Renders a drawer holding one focusable child, which stands in for the thread
 * composer that owns Escape while an edit is in progress.
 */
async function renderDrawer({ escapeYieldsToContent }) {
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { CoverDrawer } = await import("./CoverDrawer.tsx");

  const closes = [];
  const view = render(
    React.createElement(
      CoverDrawer,
      {
        ariaLabel: "Thread",
        escapeYieldsToContent,
        onClose: () => closes.push("close"),
        scrimLabel: "Back to #general",
        testId: "cover-drawer",
      },
      React.createElement("input", { "data-testid": "thread-composer" }),
    ),
  );

  return { closes, view };
}

/**
 * Renders the replacement window the way `ChannelPane` produces it: one
 * `AnimatePresence` whose keyed child is swapped, so the outgoing drawer stays
 * mounted in its exit phase while the successor mounts alongside it.
 *
 * Both are the real primitive, and the presence wrapper is real too, because the
 * bug lives in the interaction between two instances under `AnimatePresence` —
 * a harness that renders them as two independent trees reports both as present
 * and cannot see it.
 */
async function renderReplacement({ successorOwnsEscape }) {
  const React = await import("react");
  const { act, render } = await import("@testing-library/react");
  const { AnimatePresence } = await import("motion/react");
  const { CoverDrawer } = await import("./CoverDrawer.tsx");

  const events = [];

  // Stands in for the agent session panel's own `useEscapeKey`: the successor
  // drawer does not claim the key, its content handles it.
  function SuccessorContent() {
    React.useEffect(() => {
      function onKeyDown(event) {
        if (event.key === "Escape") events.push("successor-content-escape");
      }
      window.addEventListener("keydown", onKeyDown);
      return () => window.removeEventListener("keydown", onKeyDown);
    }, []);
    return React.createElement("div", { "data-testid": "successor-content" });
  }

  const outgoing = React.createElement(
    CoverDrawer,
    {
      ariaLabel: "Thread",
      key: "outgoing",
      onClose: () => events.push("outgoing-close"),
      scrimLabel: "Back to #general",
      testId: "outgoing-drawer",
    },
    React.createElement("div", null, "thread"),
  );
  const successor = React.createElement(
    CoverDrawer,
    {
      ariaLabel: "Agent activity",
      key: "successor",
      onClose: () => events.push("successor-close"),
      ownsEscape: successorOwnsEscape,
      scrimLabel: "Back to #general",
      testId: "successor-drawer",
    },
    React.createElement(SuccessorContent),
  );

  const view = render(React.createElement(AnimatePresence, null, outgoing));
  await act(async () => {
    view.rerender(React.createElement(AnimatePresence, null, successor));
  });

  // Both are mounted: the outgoing drawer is held through its exit animation.
  // This is the ~210ms window an rAF probe measures in the browser, and it is
  // the precondition for the assertions below — without it they prove nothing.
  assert.equal(
    dom.window.document.querySelectorAll('[data-testid="outgoing-drawer"]')
      .length,
    1,
  );
  assert.equal(
    dom.window.document.querySelectorAll('[data-testid="successor-drawer"]')
      .length,
    1,
  );

  return { events, view };
}

function pressEscapeOn(element) {
  element.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "Escape",
    }),
  );
}

/**
 * Renders the replacement window in the shape production actually has it: each
 * drawer holds a panel using the real `useEscapeKey`, which is how both the
 * thread and the agent session panels take the key.
 *
 * The synthetic harness above cannot see the second half of this bug. Its
 * successor listener acts on every press, but `useEscapeKey` deliberately
 * ignores an event that is already `defaultPrevented` — so an exiting *panel*
 * that still calls `preventDefault` swallows the press from a real successor
 * just as thoroughly as the exiting drawer's `stopImmediatePropagation` does,
 * one layer further down and with no cover-drawer code in the path.
 */
async function renderPanelReplacement() {
  const React = await import("react");
  const { act, render } = await import("@testing-library/react");
  const { AnimatePresence } = await import("motion/react");
  const { CoverDrawer } = await import("./CoverDrawer.tsx");
  const { useEscapeKey } = await import("@/shared/hooks/useEscapeKey.ts");

  const events = [];

  function Panel({ label, testId }) {
    useEscapeKey(
      React.useCallback(() => events.push(label), [label]),
      true,
    );
    return React.createElement("div", { "data-testid": testId });
  }

  // The thread drawer claims Escape itself; activity leaves it to its panel.
  const outgoing = React.createElement(
    CoverDrawer,
    {
      ariaLabel: "Thread",
      key: "outgoing",
      onClose: () => events.push("outgoing-drawer-close"),
      scrimLabel: "Back to #general",
      testId: "outgoing-drawer",
    },
    React.createElement(Panel, {
      label: "outgoing-panel-escape",
      testId: "outgoing-panel",
    }),
  );
  const successor = React.createElement(
    CoverDrawer,
    {
      ariaLabel: "Agent activity",
      key: "successor",
      onClose: () => events.push("successor-drawer-close"),
      ownsEscape: false,
      scrimLabel: "Back to #general",
      testId: "successor-drawer",
    },
    React.createElement(Panel, {
      label: "successor-panel-escape",
      testId: "successor-panel",
    }),
  );

  const view = render(React.createElement(AnimatePresence, null, outgoing));
  await act(async () => {
    view.rerender(React.createElement(AnimatePresence, null, successor));
  });

  assert.equal(
    dom.window.document.querySelectorAll('[data-testid="outgoing-drawer"]')
      .length,
    1,
  );
  assert.equal(
    dom.window.document.querySelectorAll('[data-testid="successor-panel"]')
      .length,
    1,
  );

  return { events, view };
}

function composer() {
  return dom.window.document.querySelector('[data-testid="thread-composer"]');
}

test("Escape inside the drawer yields to content while it owns the key", async () => {
  // The regression this guards (#6575): the drawer claims Escape in the capture
  // phase, which runs before the composer's own handler. Without the yield, one
  // press dismisses the entire drawer instead of cancelling the in-progress
  // edit, and the unsaved draft goes with it.
  const { closes } = await renderDrawer({ escapeYieldsToContent: true });

  pressEscapeOn(composer());

  assert.deepEqual(closes, []);
});

test("Escape inside the drawer closes it when content does not own the key", async () => {
  // The default: with no active edit the same press is a dismissal, so the yield
  // above must be conditional rather than a blanket exemption for the subtree.
  const { closes } = await renderDrawer({ escapeYieldsToContent: false });

  pressEscapeOn(composer());

  assert.deepEqual(closes, ["close"]);
});

test("Escape from outside the drawer closes it even while content owns the key", async () => {
  // The yield is scoped to the drawer's own subtree, so an active edit inside
  // cannot wedge the drawer open against a press from the channel behind it.
  const { closes } = await renderDrawer({ escapeYieldsToContent: true });

  pressEscapeOn(dom.window.document.body);

  assert.deepEqual(closes, ["close"]);
});

test("Escape during a replacement reaches the successor, not the exiting drawer", async () => {
  // The bug this guards: `AnimatePresence` holds the outgoing drawer mounted
  // through its exit animation (~210ms), and its capture-phase listener calls
  // `stopImmediatePropagation()`. A successor that does not claim Escape — the
  // agent activity drawer, which routes the key through its panel's own
  // `useEscapeKey` — therefore never sees the press, so the user has to press
  // Escape twice to leave a drawer that just replaced another.
  const { events, view } = await renderReplacement({
    successorOwnsEscape: false,
  });

  pressEscapeOn(
    dom.window.document.querySelector('[data-testid="successor-content"]'),
  );

  // The press belongs to the drawer holding the covered slot. The exiting
  // drawer is on its way out and must not act on it, let alone consume it.
  assert.deepEqual(events, ["successor-content-escape"]);

  view.unmount();
});

test("a claiming successor closes on a single Escape during a replacement", async () => {
  // The same window, with a successor that does claim the key (thread over
  // activity): exactly one drawer may act, and it must be the new one.
  const { events, view } = await renderReplacement({
    successorOwnsEscape: true,
  });

  pressEscapeOn(
    dom.window.document.querySelector('[data-testid="successor-content"]'),
  );

  assert.deepEqual(events, ["successor-close"]);

  view.unmount();
});

test("Escape during a panel replacement reaches the successor's panel", async () => {
  // The other half of the same bug, one layer down and with no cover-drawer
  // code in the path: `useEscapeKey` ignores an event that is already
  // `defaultPrevented`, so an exiting panel that still calls `preventDefault`
  // silently consumes the press from its successor's panel. This is the path the
  // agent activity drawer actually takes — it sets `ownsEscape={false}` and lets
  // its panel handle the key — so fixing only the drawer's claim leaves the
  // two-press bug in place.
  const { events, view } = await renderPanelReplacement();

  pressEscapeOn(
    dom.window.document.querySelector('[data-testid="successor-panel"]'),
  );

  // Exactly one handler acts, and it belongs to the arriving surface.
  assert.deepEqual(events, ["successor-panel-escape"]);

  view.unmount();
});

test("a lone panel still closes on Escape outside AnimatePresence", async () => {
  // `useEscapeKey` is used by panels that never animate out (split pane,
  // single-panel thread, profile). With no `AnimatePresence` above them there is
  // no presence context at all, and the guard must read as present rather than
  // as "not exiting yet" — otherwise it would disable Escape for every one of
  // those surfaces.
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { useEscapeKey } = await import("@/shared/hooks/useEscapeKey.ts");

  const closes = [];
  function Panel() {
    useEscapeKey(() => closes.push("close"), true);
    return React.createElement("input", { "data-testid": "thread-composer" });
  }
  render(React.createElement(Panel));

  pressEscapeOn(composer());

  assert.deepEqual(closes, ["close"]);
});

test("a lone drawer still closes on Escape under StrictMode", async () => {
  // The slot claim is taken in a layout effect, which `React.StrictMode` replays
  // as setup → cleanup → setup. Each setup takes a *new* generation, so the
  // guard above must be reading whatever the last setup stored rather than a
  // stale claim from the discarded first pass — otherwise every drawer in
  // development would ignore Escape entirely. The focus tests cannot see this:
  // they assert on restore, which the coordinator handles separately.
  const React = await import("react");
  const { render } = await import("@testing-library/react");
  const { CoverDrawer } = await import("./CoverDrawer.tsx");

  const closes = [];
  render(
    React.createElement(
      CoverDrawer,
      {
        ariaLabel: "Thread",
        onClose: () => closes.push("close"),
        scrimLabel: "Back to #general",
        testId: "strict-drawer",
      },
      React.createElement("input", { "data-testid": "thread-composer" }),
    ),
    { reactStrictMode: true },
  );

  pressEscapeOn(composer());

  assert.deepEqual(closes, ["close"]);
});
