import assert from "node:assert/strict";
import test from "node:test";

import { effectiveProjectCanvasCapabilities } from "./projectCanvasConsent.ts";
import {
  grantedProjectCanvasCapabilities,
  parseProjectCanvasChildMessage,
  parseProjectCanvasPackageDescriptor,
  parseProjectCanvasPackageDescriptorForE2e,
  parseProjectCanvasReady,
  PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT,
  PROJECT_CANVAS_LAYOUT_MIN_WIDGET_SIZE,
  PROJECT_CANVAS_MAX_INIT_MESSAGE_BYTES,
  PROJECT_CANVAS_MAX_LAYOUT_WIDGETS,
  PROJECT_CANVAS_MESSAGE_RATE_LIMIT,
  PROJECT_CANVAS_MESSAGE_RATE_WINDOW_MS,
  projectCanvasConsentCapabilities,
  ProjectCanvasMessageRateLimiter,
  selectGrantedProjectCanvasSnapshots,
} from "./projectCanvasProtocol.ts";

const LOAD_ID = "0123456789abcdef0123456789abcdef";
const NONCE = "0123456789abcdef0123456789abcdef";

function descriptor(overrides = {}) {
  return {
    capabilities: ["project.metadata.read"],
    data: { widgets: [] },
    loadId: LOAD_ID,
    nonce: NONCE,
    revision: "revision-1",
    url: `buzz-canvas://localhost/${LOAD_ID}/`,
    ...overrides,
  };
}

test("package descriptors accept only the exact canvas protocol origin and handle path", () => {
  assert.equal(
    parseProjectCanvasPackageDescriptor(descriptor()).loadId,
    LOAD_ID,
  );
  assert.equal(
    parseProjectCanvasPackageDescriptor(
      descriptor({ url: `http://buzz-canvas.localhost/${LOAD_ID}/` }),
    ).loadId,
    LOAD_ID,
  );

  for (const url of [
    `buzz-canvas://other/${LOAD_ID}/`,
    `buzz-canvas://localhost/${LOAD_ID}`,
    `buzz-canvas://localhost/${LOAD_ID}/asset.js`,
    `buzz-canvas://localhost/${LOAD_ID}/?token=1`,
    `buzz-canvas://localhost/${LOAD_ID}/#fragment`,
    `buzz-canvas://user@localhost/${LOAD_ID}/`,
    `http://buzz-canvas.localhost:80/${LOAD_ID}/`,
    "https://example.com/canvas",
    "data:text/html,canvas",
  ]) {
    assert.throws(() =>
      parseProjectCanvasPackageDescriptor(descriptor({ url })),
    );
  }
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(
      descriptor({ loadId: "fedcba9876543210fedcba9876543210" }),
    ),
  );
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(
      descriptor({ loadId: LOAD_ID.toUpperCase() }),
    ),
  );
});

test("data URLs are available only through the explicit E2E parser", () => {
  const value = descriptor({ url: "data:text/html,canvas" });
  assert.throws(() => parseProjectCanvasPackageDescriptor(value));
  assert.equal(
    parseProjectCanvasPackageDescriptorForE2e(value).loadId,
    LOAD_ID,
  );
});

test("package data must be bounded JSON", () => {
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(
      descriptor({ data: { value: Number.POSITIVE_INFINITY } }),
    ),
  );
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(
      descriptor({ data: { value: new Date() } }),
    ),
  );
  let nested = {};
  for (let index = 0; index < 32; index += 1) nested = { nested };
  assert.equal(
    parseProjectCanvasPackageDescriptor(descriptor({ data: nested })).loadId,
    LOAD_ID,
  );
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(descriptor({ data: { nested } })),
  );
  assert.equal(
    parseProjectCanvasPackageDescriptor(
      descriptor({ data: Array.from({ length: 9_999 }, () => 0) }),
    ).loadId,
    LOAD_ID,
  );
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(
      descriptor({ data: Array.from({ length: 10_000 }, () => 0) }),
    ),
  );
});

test("native-sized package data leaves bounded headroom for host snapshots", () => {
  const nativeSizedData = { value: "x".repeat(256 * 1_024) };
  assert.equal(
    parseProjectCanvasPackageDescriptor(descriptor({ data: nativeSizedData }))
      .loadId,
    LOAD_ID,
  );
  assert.throws(() =>
    parseProjectCanvasPackageDescriptor(
      descriptor({ data: { value: "x".repeat(320 * 1_024) } }),
    ),
  );
  assert.equal(
    new TextEncoder().encode(
      JSON.stringify({
        data: nativeSizedData,
        snapshots: { avatars: "x".repeat(1_600 * 1_024) },
      }),
    ).byteLength < PROJECT_CANVAS_MAX_INIT_MESSAGE_BYTES,
    true,
  );
});

test("ready messages require the expected version, nonce, and exact shape", () => {
  const ready = {
    nonce: NONCE,
    protocolVersion: 1,
    type: "canvas.ready",
  };
  assert.deepEqual(parseProjectCanvasReady(ready, NONCE), ready);
  assert.equal(parseProjectCanvasReady(ready, "different-nonce-value"), null);
  assert.equal(
    parseProjectCanvasReady({ ...ready, protocolVersion: 2 }, NONCE),
    null,
  );
  assert.equal(
    parseProjectCanvasReady({ ...ready, projectId: "spoof" }, NONCE),
    null,
  );
});

test("child messages are bound to the native load before action fields are accepted", () => {
  const rendered = {
    dashboard: "home",
    loadId: LOAD_ID,
    nonce: NONCE,
    protocolVersion: 1,
    type: "canvas.rendered",
  };
  assert.deepEqual(
    parseProjectCanvasChildMessage(rendered, {
      loadId: LOAD_ID,
      nonce: NONCE,
    }),
    rendered,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...rendered, loadId: "fedcba9876543210fedcba9876543210" },
      { loadId: LOAD_ID, nonce: NONCE },
    ),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...rendered, nonce: "fedcba9876543210fedcba9876543210" },
      { loadId: LOAD_ID, nonce: NONCE },
    ),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...rendered, protocolVersion: 2 },
      { loadId: LOAD_ID, nonce: NONCE },
    ),
    null,
  );
});

test("capabilities are intersected with the fixed host set", () => {
  assert.deepEqual(
    grantedProjectCanvasCapabilities([
      "project.metadata.read",
      "network",
      "project.channels.read",
      "project.metadata.read",
      "project.reviews.read",
      "filesystem",
      "project.tasks.read",
      "project.people.read",
      "project.tasks.write",
      "app.open",
      "app.dm.send",
    ]),
    [
      "project.metadata.read",
      "project.channels.read",
      "project.reviews.read",
      "project.tasks.read",
      "project.people.read",
      "project.tasks.write",
      "app.open",
      "app.dm.send",
    ],
  );
});

test("consent capabilities are the consequential subset of a request", () => {
  assert.deepEqual(
    projectCanvasConsentCapabilities([
      "project.metadata.read",
      "project.tasks.write",
      "app.open",
      "app.dm.send",
      "network",
    ]),
    ["project.tasks.write", "app.open", "app.dm.send"],
  );
  assert.deepEqual(
    projectCanvasConsentCapabilities(["project.metadata.read"]),
    [],
  );
});

test("effective capabilities withhold consequential ones until approval", () => {
  const requested = [
    "project.metadata.read",
    "project.tasks.write",
    "app.open",
    "app.dm.send",
  ];
  assert.deepEqual(effectiveProjectCanvasCapabilities(requested, null), [
    "project.metadata.read",
  ]);
  assert.deepEqual(effectiveProjectCanvasCapabilities(requested, "denied"), [
    "project.metadata.read",
  ]);
  assert.deepEqual(effectiveProjectCanvasCapabilities(requested, "approved"), [
    "project.metadata.read",
    "project.tasks.write",
    "app.open",
    "app.dm.send",
  ]);
});

test("snapshot selection omits every capability the package was not granted", () => {
  const snapshots = {
    channels: { data: [], status: "ready" },
    project: {
      data: {
        description: "Project",
        id: "30621:owner:project",
        name: "Project",
        owner: "owner",
        repositories: [],
      },
      status: "ready",
    },
    reviews: { data: null, status: "loading" },
  };
  assert.deepEqual(
    selectGrantedProjectCanvasSnapshots(snapshots, [
      "project.metadata.read",
      "project.reviews.read",
    ]),
    {
      project: snapshots.project,
      reviews: snapshots.reviews,
    },
  );
});

test("message rate limiting uses a bounded rolling window", () => {
  const limiter = new ProjectCanvasMessageRateLimiter();
  for (let index = 0; index < PROJECT_CANVAS_MESSAGE_RATE_LIMIT; index += 1) {
    assert.equal(limiter.accept(index), true);
  }
  assert.equal(limiter.accept(PROJECT_CANVAS_MESSAGE_RATE_LIMIT), false);
  assert.equal(limiter.accept(PROJECT_CANVAS_MESSAGE_RATE_WINDOW_MS + 1), true);
});

test("rate limiter tiers honor their configured limit and window", () => {
  const limiter = new ProjectCanvasMessageRateLimiter(3, 10_000);
  assert.equal(limiter.accept(0), true);
  assert.equal(limiter.accept(1), true);
  assert.equal(limiter.accept(2), true);
  assert.equal(limiter.accept(3), false);
  assert.equal(limiter.accept(9_999), false);
  assert.equal(limiter.accept(10_001), true);
});

test("rpc child messages parse strictly and reject malformed ids and payloads", () => {
  const binding = { loadId: LOAD_ID, nonce: NONCE };
  const base = { loadId: LOAD_ID, nonce: NONCE, protocolVersion: 1 };
  const query = {
    ...base,
    query: { name: "project.tasks.list", params: { limit: 8 } },
    queryId: "q-1",
    type: "canvas.query",
  };
  assert.deepEqual(parseProjectCanvasChildMessage(query, binding), query);

  const subscribe = {
    ...base,
    query: { name: "project.channels.list" },
    subscriptionId: "s-1",
    type: "canvas.subscribe",
  };
  assert.deepEqual(
    parseProjectCanvasChildMessage(subscribe, binding),
    subscribe,
  );
  assert.deepEqual(
    parseProjectCanvasChildMessage(
      { ...base, subscriptionId: "s-1", type: "canvas.unsubscribe" },
      binding,
    ),
    { ...base, subscriptionId: "s-1", type: "canvas.unsubscribe" },
  );

  const command = {
    ...base,
    command: {
      name: "tasks.setStatus",
      params: { id: "a".repeat(64), status: "done" },
    },
    commandId: "c-1",
    type: "canvas.command",
  };
  assert.deepEqual(parseProjectCanvasChildMessage(command, binding), command);

  const open = {
    ...base,
    openId: "o-1",
    target: { id: "channel-1", type: "channel" },
    type: "canvas.open",
  };
  assert.deepEqual(parseProjectCanvasChildMessage(open, binding), open);

  // Malformed ids, oversized names, unknown fields, and unbounded payloads.
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...query, queryId: "bad id with spaces" },
      binding,
    ),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...query, queryId: "q".repeat(65) },
      binding,
    ),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...query, query: { name: "n".repeat(65) } },
      binding,
    ),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage({ ...query, extra: true }, binding),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      {
        ...query,
        query: {
          name: "project.tasks.list",
          params: { value: Number.POSITIVE_INFINITY },
        },
      },
      binding,
    ),
    null,
  );
  assert.equal(
    parseProjectCanvasChildMessage(
      { ...open, target: { date: new Date() } },
      binding,
    ),
    null,
  );
});

test("layout messages carry bounded coordinates for well-formed widget ids", () => {
  const binding = { loadId: LOAD_ID, nonce: NONCE };
  const base = { loadId: LOAD_ID, nonce: NONCE, protocolVersion: 1 };
  const layout = {
    ...base,
    dashboard: "dev",
    pan: { x: 24, y: -48 },
    type: "canvas.layout",
    widgets: { "active-channels": { x: 0, y: 384 } },
  };
  assert.deepEqual(parseProjectCanvasChildMessage(layout, binding), layout);
  assert.deepEqual(
    parseProjectCanvasChildMessage({ ...layout, pan: null }, binding),
    { ...layout, pan: null },
  );
  assert.deepEqual(
    parseProjectCanvasChildMessage({ ...layout, widgets: {} }, binding),
    { ...layout, widgets: {} },
  );

  const widgets = {};
  for (let index = 0; index <= PROJECT_CANVAS_MAX_LAYOUT_WIDGETS; index += 1) {
    widgets[`widget-${index}`] = { x: index, y: index };
  }
  assert.equal(
    parseProjectCanvasChildMessage({ ...layout, widgets }, binding),
    null,
  );

  for (const invalid of [
    { dashboard: "" },
    { dashboard: "d".repeat(129) },
    { pan: { x: 0 } },
    { pan: { x: 0, y: 0, z: 0 } },
    { pan: { x: Number.NaN, y: 0 } },
    { pan: { x: Number.POSITIVE_INFINITY, y: 0 } },
    { pan: { x: PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT + 1, y: 0 } },
    { pan: { x: 0, y: -PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT - 1 } },
    { widgets: { "bad id": { x: 0, y: 0 } } },
    { widgets: { ["w".repeat(129)]: { x: 0, y: 0 } } },
    { widgets: { widget: { x: "0", y: 0 } } },
    { widgets: { widget: null } },
    { widgets: null },
    { extra: true },
  ]) {
    assert.equal(
      parseProjectCanvasChildMessage({ ...layout, ...invalid }, binding),
      null,
      `expected ${JSON.stringify(invalid)} to be rejected`,
    );
  }

  assert.equal(
    parseProjectCanvasChildMessage(
      { ...layout, loadId: "fedcba9876543210fedcba9876543210" },
      binding,
    ),
    null,
  );
});

test("layout messages carry bounded optional size overrides", () => {
  const binding = { loadId: LOAD_ID, nonce: NONCE };
  const layout = {
    dashboard: "dev",
    loadId: LOAD_ID,
    nonce: NONCE,
    pan: null,
    protocolVersion: 1,
    type: "canvas.layout",
    widgets: {},
  };
  // Packages predating size persistence omit `sizes` and stay valid.
  assert.deepEqual(parseProjectCanvasChildMessage(layout, binding), layout);

  const sized = {
    ...layout,
    sizes: {
      reviews: {
        height: PROJECT_CANVAS_LAYOUT_MIN_WIDGET_SIZE,
        width: PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT,
      },
      tasks: { height: 288, width: 336 },
    },
  };
  assert.deepEqual(parseProjectCanvasChildMessage(sized, binding), sized);

  const sizes = {};
  for (let index = 0; index <= PROJECT_CANVAS_MAX_LAYOUT_WIDGETS; index += 1) {
    sizes[`widget-${index}`] = { height: 144, width: 192 };
  }
  assert.equal(
    parseProjectCanvasChildMessage({ ...layout, sizes }, binding),
    null,
  );

  for (const invalid of [
    { sizes: null },
    { sizes: { "bad id": { height: 144, width: 192 } } },
    { sizes: { widget: { width: 192 } } },
    { sizes: { widget: { height: 144, width: 192, depth: 1 } } },
    { sizes: { widget: { height: 144, width: "192" } } },
    { sizes: { widget: { height: Number.NaN, width: 192 } } },
    { sizes: { widget: { height: Number.POSITIVE_INFINITY, width: 192 } } },
    {
      sizes: {
        widget: {
          height: PROJECT_CANVAS_LAYOUT_MIN_WIDGET_SIZE - 1,
          width: 192,
        },
      },
    },
    { sizes: { widget: { height: 0, width: 192 } } },
    { sizes: { widget: { height: -144, width: 192 } } },
    {
      sizes: {
        widget: {
          height: 144,
          width: PROJECT_CANVAS_LAYOUT_COORDINATE_LIMIT + 1,
        },
      },
    },
  ]) {
    assert.equal(
      parseProjectCanvasChildMessage({ ...layout, ...invalid }, binding),
      null,
      `expected ${JSON.stringify(invalid)} to be rejected`,
    );
  }
});
