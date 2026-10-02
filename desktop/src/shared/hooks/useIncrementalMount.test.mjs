import assert from "node:assert/strict";
import test from "node:test";

import { countGroupedRows, sliceGroupedRows } from "./useIncrementalMount.ts";

function group(id, rowCount) {
  return { id, rows: Array.from({ length: rowCount }, (_, i) => `${id}-${i}`) };
}

test("sliceGroupedRows trims across group boundaries in order", () => {
  const groups = [group("a", 3), group("b", 4), group("c", 2)];

  assert.deepEqual(
    sliceGroupedRows(groups, 5).map((g) => [g.id, g.rows.length]),
    [
      ["a", 3],
      ["b", 2],
    ],
  );
});

test("sliceGroupedRows keeps whole groups untouched when under budget", () => {
  const groups = [group("a", 3), group("b", 4)];
  const sliced = sliceGroupedRows(groups, 10);
  assert.equal(sliced.length, 2);
  // Fully-included groups keep their identity (no needless clones).
  assert.equal(sliced[0], groups[0]);
  assert.equal(sliced[1], groups[1]);
});

test("sliceGroupedRows with zero budget is empty; counts add up", () => {
  const groups = [group("a", 3), group("b", 4)];
  assert.deepEqual(sliceGroupedRows(groups, 0), []);
  assert.equal(countGroupedRows(groups), 7);
});

// --- Hook behavior: growth, layout gating, shrink reset -------------------

async function withHookHarness(run) {
  const { JSDOM } = await import("jsdom");
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  const originals = {
    window: globalThis.window,
    document: globalThis.document,
    navigator: Object.getOwnPropertyDescriptor(globalThis, "navigator"),
    act: globalThis.IS_REACT_ACT_ENVIRONMENT,
  };
  const frames = [];
  dom.window.requestAnimationFrame = (cb) => frames.push(cb) && frames.length;
  dom.window.cancelAnimationFrame = (id) => {
    frames[id - 1] = null;
  };
  globalThis.window = dom.window;
  globalThis.document = dom.window.document;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  try {
    const React = (await import("react")).default;
    const { act } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const { useIncrementalMount } = await import("./useIncrementalMount.ts");

    const seen = [];
    function Probe({ total, enabled }) {
      seen.push(useIncrementalMount(total, 2, 3, enabled));
      return null;
    }
    const root = createRoot(
      dom.window.document.body.appendChild(
        dom.window.document.createElement("div"),
      ),
    );
    const render = async (total, enabled = true) => {
      await act(async () => {
        root.render(React.createElement(Probe, { enabled, total }));
      });
    };
    const flushFrames = async () => {
      // Growth chains one rAF per step; run until quiescent.
      for (let i = 0; i < 20 && frames.some(Boolean); i += 1) {
        const pending = frames.splice(0, frames.length).filter(Boolean);
        await act(async () => {
          for (const cb of pending) cb();
        });
      }
    };
    await run({ render, flushFrames, seen, root, act });
    await act(async () => root.unmount());
  } finally {
    globalThis.window = originals.window;
    globalThis.document = originals.document;
    if (originals.navigator) {
      Object.defineProperty(globalThis, "navigator", originals.navigator);
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = originals.act;
  }
}

test("hook grows stepwise to total and persists across data updates", async () => {
  await withHookHarness(async ({ render, flushFrames, seen }) => {
    await render(8);
    assert.equal(seen.at(-1), 2); // initial window paints first
    await flushFrames();
    assert.equal(seen.at(-1), 8); // grown 2→5→8
    await render(8); // in-place update: stays fully mounted
    assert.equal(seen.at(-1), 8);
  });
});

test("hook holds at initial while disabled and grows on activation", async () => {
  await withHookHarness(async ({ render, flushFrames, seen }) => {
    await render(9, false);
    await flushFrames();
    assert.equal(seen.at(-1), 2, "disabled layout must not grow");
    await render(9, true);
    await flushFrames();
    assert.equal(seen.at(-1), 9);
    // Deactivating resets to the initial window for the next activation.
    await render(9, false);
    await flushFrames();
    assert.equal(seen.at(-1), 2);
  });
});

test("hook snaps down on total shrink and re-grows stepwise on restore", async () => {
  await withHookHarness(async ({ render, flushFrames, seen }) => {
    await render(9);
    await flushFrames();
    assert.equal(seen.at(-1), 9);
    await render(4); // scope narrowed
    await flushFrames();
    assert.equal(seen.at(-1), 4);
    await render(9); // scope restored: no one-shot bulk mount
    assert.equal(seen.at(-1), 4);
    await flushFrames();
    assert.equal(seen.at(-1), 9);
  });
});

test("list dwell then grid switch: first enabled commit stays at the initial window", async () => {
  await withHookHarness(async ({ render, flushFrames, seen }) => {
    // Dwell in list layout (counter disabled) long enough that an ungated
    // counter would have grown to the full collection.
    await render(20, false);
    await flushFrames();
    assert.equal(seen.at(-1), 2, "dormant counter must hold at initial");
    // Switch to grid: the FIRST grid commit must mount only the initial
    // window — never the whole collection in one commit.
    await render(20, true);
    assert.equal(seen.at(-1), 2, "first grid commit stays at initial");
    await flushFrames();
    assert.equal(seen.at(-1), 20, "then grows stepwise to the total");
  });
});
