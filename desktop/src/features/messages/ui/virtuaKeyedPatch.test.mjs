import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import vm from "node:vm";

const require = createRequire(import.meta.url);
// Run the installed, patched distribution's real store, not stale source-map
// sources or a reimplementation. Both published module formats must agree.
// Load-bearing ordering in action 10: resolve the anchor and oldOffset from
// PRE-mutation sizes/offsets, then remap keys and replace the arrays. The mixed
// prepend/delete test below must fail if anchor selection moves after the swap.
for (const format of ["esm", "cjs"]) {
  const file = require
    .resolve("virtua")
    .replace(/index\.cjs$/, format === "esm" ? "index.js" : "index.cjs");
  const source = readFileSync(file, "utf8");
  const from = source.indexOf(
    format === "esm" ? "const u = null" : "const r = null",
  );
  const to =
    source.indexOf(
      format === "esm" ? "}, H = setTimeout" : "}, k = setTimeout",
      from,
    ) + 1;
  const createStore = vm.runInNewContext(
    `${source.slice(from, to)}; ${format === "esm" ? "E" : "I"}`,
    {
      navigator: { userAgent: "", platform: "", maxTouchPoints: 0 },
    },
  );
  const bindingEnd =
    source.indexOf(
      format === "esm" ? "}, J = (e, t, o)" : "}, C = (e, t, o)",
      to,
    ) + 1;
  const bindInput = vm.runInNewContext(
    `${source.slice(from, bindingEnd)}; ${format === "esm" ? "B" : "T"}`,
    {
      navigator: { userAgent: "", platform: "", maxTouchPoints: 0 },
      setTimeout,
      clearTimeout,
    },
  );
  const update = (store, action, value) =>
    store[format === "esm" ? "B" : "q"](action, value);
  const cache = (store) => Array.from(store[format === "esm" ? "_" : "S"]()[0]);
  const make = () => {
    const store = createStore(4, [40, 80, 120, 160]);
    update(store, 4, 100);
    update(store, 1, 135); // 15px inside c
    update(store, 2);
    return store;
  };
  test(`${format}: mixed prepend/delete preserves measured key sizes and pixel offset`, () => {
    const store = make();
    update(store, 10, [
      ["a", "b", "c", "d"],
      ["new", "a", "c", "d"],
      [60, 50, 50, 50],
    ]);
    assert.deepEqual(cache(store), [60, 40, 120, 160]);
    assert.deepEqual(Array.from(store.H()), [-20, true]); // c:120 ->100
    update(store, 1, 115);
    update(store, 3, [
      [0, 90],
      [2, 200],
      [3, 170],
    ]);
    assert.deepEqual(
      Array.from(store.H()),
      [30, true],
      "only sizes above the same visible row compensate",
    );
  });
  test(`${format}: removed anchor selects next surviving neighbor at its old offset`, () => {
    const store = make();
    update(store, 10, [
      ["a", "b", "c", "d"],
      ["new", "a", "d"],
      [60, 50, 50],
    ]);
    assert.deepEqual(cache(store), [60, 40, 160]);
    assert.deepEqual(Array.from(store.H()), [-140, true]); // d:240 ->100
  });
  test(`${format}: same-length marker replacement does not retain positional sizes`, () => {
    const store = make();
    update(store, 10, [
      ["a", "b", "c", "d"],
      ["a", "marker", "c", "d"],
      [50, 20, 50, 50],
    ]);
    assert.deepEqual(cache(store), [40, 20, 120, 160]);
    assert.deepEqual(Array.from(store.H()), [-60, true]);
  });
  test(`${format}: keyed prefix measures above the reading row, not its changed grouping or tail`, () => {
    const store = createStore(3, [68, 48, 96]);
    update(store, 4, 100);
    update(store, 1, 0);
    update(store, 5, [5, true, [60, 56, 68, 48, 96], true]);
    assert.deepEqual(Array.from(store.H()), [116, true]);
    update(store, 1, 116);
    // DM intro/divider grow; the formerly-first message loses its author
    // header after a prepend. Its top must not move when its own size shrinks.
    update(store, 3, [
      [0, 100],
      [1, 62],
      [2, 48],
      [4, 120],
    ]);
    assert.deepEqual(Array.from(store.H()), [46, true]);
  });
  test(`${format}: unkeyed scalar and default estimates support prepend and append`, () => {
    for (const estimate of [undefined, 64]) {
      const store = createStore(3, estimate);
      update(store, 4, 100);
      update(store, 5, [5, true, estimate, false]);
      assert.equal(store.H()[0], 2 * (estimate ?? 40));
      update(store, 5, [6, false, estimate, false]);
      assert.equal(store.H()[0], 0);
      assert.equal(cache(store).length, 6);
    }
  });
  test(`${format}: legacy unkeyed shift still compensates all retained sizes`, () => {
    const store = createStore(3, [68, 48, 96]);
    update(store, 4, 100);
    update(store, 5, [5, true, [60, 56, 68, 48, 96]]);
    store.H();
    update(store, 1, 116);
    update(store, 3, [
      [0, 100],
      [1, 62],
      [2, 48],
      [4, 120],
    ]);
    assert.deepEqual(Array.from(store.H()), [50, true]);
  });
  test(`${format}: late measurements after scroll end preserve the resting viewport`, () => {
    const store = make();
    update(store, 10, [
      ["a", "b", "c", "d"],
      ["new", "a", "c", "d"],
      [60, 50, 50, 50],
    ]);
    store.H();
    update(store, 1, 115);
    update(store, 2);
    update(store, 3, [
      [0, 90],
      [2, 200],
      [3, 170],
    ]);
    assert.deepEqual(Array.from(store.H()), [30, false]);
  });
  test(`${format}: removing the tail anchor falls back to its previous surviving key`, () => {
    const store = make();
    update(store, 1, 250);
    update(store, 2);
    update(store, 10, [
      ["a", "b", "c", "d"],
      ["new", "a", "b"],
      [60, 50, 50],
    ]);
    assert.deepEqual(Array.from(store.H()), [60, true]);
    assert.deepEqual(cache(store), [60, 40, 80]);
  });
  test(`${format}: real input bindings retire anchoring while idle and ignore zoom/editing`, () => {
    const handlers = new Map();
    const target = {
      addEventListener: (name, handler) => handlers.set(name, handler),
      removeEventListener: (name) => handlers.delete(name),
    };
    const editable = { closest: () => ({}) };
    const events = [
      ["wheel", { deltaY: -1 }, true],
      ["wheel", { deltaY: -1, ctrlKey: true }, false],
      ["wheel", { deltaY: 0 }, false],
      ["touchstart", {}, true],
      ["pointerdown", { target }, true],
      ["keydown", { key: "PageUp", target }, true],
      ["keydown", { key: "Home", target: editable }, false],
      ["keydown", { key: "ArrowUp", target, ctrlKey: true }, false],
    ];
    for (const [name, event, retires] of events) {
      const store = make();
      update(store, 10, [
        ["a", "b", "c", "d"],
        ["new", "a", "c", "d"],
        [60, 50, 50, 50],
      ]);
      store.H();
      assert.equal(store.M(), false);
      const binding = bindInput(
        store,
        target,
        false,
        () => 115,
        () => {},
      );
      handlers.get(name)(event);
      // Moving the reader into the inserted row means a new measurement of
      // row a must not drag them back to the retired c anchor.
      update(store, 1, 20);
      update(store, 3, [[1, 100]]);
      assert.deepEqual(
        Array.from(store.H()),
        retires ? [0, false] : [60, true],
        `${name}: ${JSON.stringify(event)}`,
      );
      binding[format === "esm" ? "A" : "J"]();
      assert.equal(handlers.size, 0);
    }
  });
  test(`${format}: reader input retires keyed measurement intent`, () => {
    const store = make();
    update(store, 10, [
      ["a", "b", "c", "d"],
      ["new", "a", "c", "d"],
      [60, 50, 50, 50],
    ]);
    store.H();
    update(store, 9);
    update(store, 1, 20);
    update(store, 3, [[1, 100]]);
    assert.deepEqual(Array.from(store.H()), [0, false]);
  });
}
