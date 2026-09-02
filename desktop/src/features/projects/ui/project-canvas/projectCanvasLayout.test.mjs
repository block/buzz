import assert from "node:assert/strict";
import test from "node:test";

import {
  PROJECT_CANVAS_MAX_LAYOUT_RECORD_BYTES,
  PROJECT_CANVAS_MAX_STORED_LAYOUT_DASHBOARDS,
  projectCanvasLayoutStorageKey,
  readProjectCanvasLayouts,
  writeProjectCanvasDashboardLayout,
} from "./projectCanvasLayout.ts";
import { PROJECT_CANVAS_MAX_LAYOUT_WIDGETS } from "./projectCanvasProtocol.ts";

const storage = new Map();
if (typeof globalThis.window === "undefined") {
  globalThis.window = {
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      removeItem: (key) => storage.delete(key),
      setItem: (key, value) => storage.set(key, String(value)),
    },
  };
}

const COMMUNITY = "wss://relay.example";
const PROJECT = "30621:owner:proj";
const KEY = projectCanvasLayoutStorageKey(COMMUNITY, PROJECT);

test.beforeEach(() => {
  storage.clear();
});

function write(dashboard, layout) {
  writeProjectCanvasDashboardLayout(COMMUNITY, PROJECT, dashboard, layout);
}

function read() {
  return readProjectCanvasLayouts(COMMUNITY, PROJECT);
}

test("layouts round-trip per dashboard and stay scoped to their binding", () => {
  write("dev", {
    pan: { x: -48, y: 96 },
    sizes: { tasks: { height: 312, width: 360 } },
    widgets: { tasks: { x: 24, y: 48 } },
  });
  write("home", {
    pan: null,
    sizes: {},
    widgets: { chores: { x: 0, y: 24 } },
  });

  assert.deepEqual(read(), {
    dev: {
      pan: { x: -48, y: 96 },
      sizes: { tasks: { height: 312, width: 360 } },
      widgets: { tasks: { x: 24, y: 48 } },
    },
    home: { pan: null, sizes: {}, widgets: { chores: { x: 0, y: 24 } } },
  });
  assert.deepEqual(
    readProjectCanvasLayouts("wss://other.example", PROJECT),
    {},
  );
  assert.deepEqual(
    readProjectCanvasLayouts(COMMUNITY, "30621:owner:other"),
    {},
  );
});

test("a dashboard write replaces its entry wholesale, pruning stale widget ids", () => {
  write("dev", {
    pan: null,
    sizes: {
      removed: { height: 168, width: 216 },
      tasks: { height: 192, width: 240 },
    },
    widgets: { removed: { x: 24, y: 24 }, tasks: { x: 48, y: 48 } },
  });
  write("dev", {
    pan: null,
    sizes: { tasks: { height: 216, width: 264 } },
    widgets: { tasks: { x: 72, y: 72 } },
  });

  assert.deepEqual(read().dev, {
    pan: null,
    sizes: { tasks: { height: 216, width: 264 } },
    widgets: { tasks: { x: 72, y: 72 } },
  });
});

test("a size-only layout is stored, and resizing never pins a position", () => {
  write("dev", {
    pan: null,
    sizes: { tasks: { height: 312, width: 360 } },
    widgets: {},
  });
  assert.deepEqual(read().dev, {
    pan: null,
    sizes: { tasks: { height: 312, width: 360 } },
    widgets: {},
  });
});

test("an empty layout deletes its dashboard, and the last one clears the record", () => {
  write("dev", { pan: null, sizes: {}, widgets: { tasks: { x: 24, y: 24 } } });
  write("home", { pan: { x: 0, y: 0 }, sizes: {}, widgets: {} });
  write("dev", { pan: null, sizes: {}, widgets: {} });

  assert.deepEqual(Object.keys(read()), ["home"]);
  write("home", { pan: null, sizes: {}, widgets: {} });
  assert.deepEqual(read(), {});
  assert.equal(storage.has(KEY), false);
});

test("corrupt, oversized, and malformed stored records read as no layouts", () => {
  for (const raw of [
    "{not json",
    "null",
    '{"dashboards":{}}',
    '{"version":1}',
    JSON.stringify({
      dashboards: [{ dashboard: "dev", pan: null, widgets: "nope" }],
    }),
  ]) {
    storage.set(KEY, raw);
    assert.deepEqual(read(), {}, `expected ${raw} to read as no layouts`);
  }

  storage.set(
    KEY,
    JSON.stringify({
      dashboards: [
        { dashboard: "dev", pan: { x: "0", y: 0 }, widgets: {} },
        { dashboard: 7, pan: null, widgets: { tasks: { x: 0, y: 24 } } },
        {
          dashboard: "home",
          pan: null,
          sizes: {
            "bad id": { height: 144, width: 192 },
            fine: { height: 192, width: 240 },
            flat: { height: 0, width: 240 },
            huge: { height: 144, width: Number.MAX_VALUE },
            partial: { width: 240 },
            tiny: { height: 4, width: 4 },
          },
          widgets: {
            "bad id": { x: 0, y: 0 },
            fine: { x: 24, y: 0 },
            infinite: { x: Number.MAX_VALUE, y: 0 },
          },
        },
      ],
      version: 1,
    }),
  );
  assert.deepEqual(read(), {
    home: {
      pan: null,
      sizes: { fine: { height: 192, width: 240 } },
      widgets: { fine: { x: 24, y: 0 } },
    },
  });

  storage.set(KEY, "x".repeat(PROJECT_CANVAS_MAX_LAYOUT_RECORD_BYTES + 1));
  assert.deepEqual(read(), {});
});

test("stored dashboards and widgets are capped, dropping the least recently written", () => {
  const total = PROJECT_CANVAS_MAX_STORED_LAYOUT_DASHBOARDS + 4;
  for (let index = 0; index < total; index += 1) {
    write(`dashboard-${index}`, {
      pan: null,
      widgets: { tasks: { x: index, y: 0 } },
    });
  }
  const stored = Object.keys(read());
  assert.equal(stored.length, PROJECT_CANVAS_MAX_STORED_LAYOUT_DASHBOARDS);
  assert.equal(stored.includes("dashboard-0"), false);
  assert.equal(stored.includes(`dashboard-${total - 1}`), true);

  const widgets = {};
  const sizes = {};
  for (let index = 0; index <= PROJECT_CANVAS_MAX_LAYOUT_WIDGETS; index += 1) {
    widgets[`widget-${index}`] = { x: index, y: index };
    sizes[`widget-${index}`] = { height: 144 + index, width: 192 + index };
  }
  write("wide", { pan: null, sizes, widgets });
  assert.equal(
    Object.keys(read().wide.widgets).length,
    PROJECT_CANVAS_MAX_LAYOUT_WIDGETS,
  );
  assert.equal(
    Object.keys(read().wide.sizes).length,
    PROJECT_CANVAS_MAX_LAYOUT_WIDGETS,
  );
});

test("a widget named __proto__ is stored as data, never as a prototype", () => {
  // A computed key defines an own property; a literal would set the prototype.
  write("dev", {
    pan: null,
    sizes: { ["__proto__"]: { height: 144, width: 192 } },
    widgets: { ["__proto__"]: { x: 24, y: 24 } },
  });
  const layouts = read();
  assert.deepEqual(Object.keys(layouts.dev.widgets), ["__proto__"]);
  assert.deepEqual(Object.keys(layouts.dev.sizes), ["__proto__"]);
  assert.equal(Object.getPrototypeOf(layouts.dev.widgets), Object.prototype);
  assert.equal(Object.getPrototypeOf(layouts.dev.sizes), Object.prototype);
  assert.equal({}.x, undefined);
  assert.equal({}.width, undefined);
});
