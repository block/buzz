import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFile, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDirectory = path.dirname(fileURLToPath(import.meta.url));
const templateCandidates = [
  path.join(testDirectory, "project-canvas-template"),
  path.resolve(
    testDirectory,
    "../../../../../src-tauri/resources/project-canvas-template",
  ),
];
const templateDirectory = templateCandidates.find((candidate) =>
  existsSync(path.join(candidate, "manifest.json")),
);

if (!templateDirectory) {
  throw new Error("Could not find the project Canvas template fixture.");
}

const desktopRoot = existsSync(
  path.resolve(process.cwd(), "desktop/package.json"),
)
  ? path.resolve(process.cwd(), "desktop")
  : path.resolve(templateDirectory, "../../..");
const require = createRequire(
  pathToFileURL(path.join(desktopRoot, "package.json")),
);
const { JSDOM } = require("jsdom");

async function readJson(relativePath) {
  return JSON.parse(
    await readFile(path.join(templateDirectory, relativePath), "utf8"),
  );
}

async function createCanvasHarness() {
  const manifest = await readJson("manifest.json");
  const fixtureData = await readJson(manifest.data);
  const sent = [];
  // A real MessagePort dispatches to every listener in registration order —
  // the host-owned SDK registers first, then canvas.js. Mirror that here.
  const listeners = [];
  let started = false;
  const port = {
    addEventListener(type, nextListener) {
      if (type === "message") listeners.push(nextListener);
    },
    emit(data) {
      assert.ok(listeners.length > 0, "canvas registered its port listener");
      for (const listener of [...listeners]) listener({ data });
    },
    postMessage(message) {
      sent.push(message);
    },
    start() {
      started = true;
    },
  };
  const dom = new JSDOM('<main id="canvas-root"></main>', {
    runScripts: "outside-only",
    url: "buzz-canvas://localhost/template/",
  });
  Object.defineProperty(dom.window, "buzzCanvas", {
    configurable: false,
    value: Object.freeze({
      packageBaseUrl: "buzz-canvas://localhost/template/package/",
      port,
      protocolVersion: 1,
      sdk: {},
    }),
    writable: false,
  });
  // The bootstrap loads the host-owned SDK before every package script.
  dom.window.eval(
    await readFile(
      path.join(desktopRoot, "src-tauri/src/project_canvas_package/sdk.js"),
      "utf8",
    ),
  );
  for (const scriptPath of manifest.scripts) {
    dom.window.eval(
      await readFile(path.join(templateDirectory, scriptPath), "utf8"),
    );
  }
  return { dom, fixtureData, manifest, port, sent, started: () => started };
}

const ALL_CAPABILITIES = [
  "project.metadata.read",
  "project.channels.read",
  "project.reviews.read",
  "project.tasks.read",
  "project.people.read",
  "project.tasks.write",
  "app.open",
  "app.dm.send",
];

function initMessage(fixtureData, overrides = {}) {
  return {
    canvasId: "canvas-1",
    capabilities: ALL_CAPABILITIES,
    data: fixtureData,
    loadId: "load-1",
    mode: "preview",
    nonce: "nonce-1",
    project: { id: "project-1", name: "my-dev-team" },
    protocolVersion: 1,
    type: "host.init",
    ...overrides,
  };
}

test("template manifest declares only local ordered resources and read capabilities", async () => {
  const manifest = await readJson("manifest.json");
  assert.deepEqual(manifest, {
    capabilities: ALL_CAPABILITIES,
    data: "data/dashboards.json",
    format: "buzz-project-canvas",
    protocolVersion: 1,
    scripts: [
      "widgets/home.js",
      "widgets/dev-team.js",
      "widgets/support.js",
      "canvas.js",
    ],
    styles: [
      "styles/base.css",
      "styles/home.css",
      "styles/dev-team.css",
      "styles/support.css",
      "styles/overlays.css",
    ],
  });

  for (const relativePath of [
    manifest.data,
    ...manifest.scripts,
    ...manifest.styles,
  ]) {
    assert.equal(path.isAbsolute(relativePath), false);
    assert.equal(relativePath.split("/").includes(".."), false);
    assert.equal(
      (await stat(path.join(templateDirectory, relativePath))).isFile(),
      true,
    );
  }
  assert.ok(manifest.scripts.every((script) => !script.startsWith("http")));
  assert.ok(manifest.styles.every((style) => !style.startsWith("http")));
});

test("fixture assets are self-contained and every presentation file stays bounded", async () => {
  const manifest = await readJson("manifest.json");
  const data = await readJson(manifest.data);
  const serializedData = JSON.stringify(data);
  const assetReferences = [
    ...serializedData.matchAll(/assets\/[a-z0-9.-]+/g),
  ].map(([match]) => match);
  assert.ok(assetReferences.length >= 7);
  for (const relativePath of new Set(assetReferences)) {
    assert.equal(
      (await stat(path.join(templateDirectory, relativePath))).isFile(),
      true,
    );
  }

  for (const relativePath of [...manifest.scripts, ...manifest.styles]) {
    const source = await readFile(
      path.join(templateDirectory, relativePath),
      "utf8",
    );
    assert.ok(
      source.split(/\r?\n/).length < 850,
      `${relativePath} is too large`,
    );
    assert.doesNotMatch(source, /<\/script|<\/style/i);
    assert.doesNotMatch(source, /\b(fetch|XMLHttpRequest|WebSocket)\s*\(/);
    assert.doesNotMatch(source, /window\.parent|__TAURI__|\binvoke\s*\(/);
    assert.doesNotMatch(
      source.replaceAll("http://www.w3.org/2000/svg", ""),
      /https?:\/\//,
    );
  }
});

test("package starts the paused host port and renders all named dashboards", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  assert.equal(harness.started(), true);
  assert.deepEqual(harness.sent, []);

  harness.port.emit(initMessage(harness.fixtureData));
  assert.equal(
    document.querySelector("[data-testid='project-widget-canvas']")?.dataset
      .projectDashboard,
    "dev",
  );
  assert.ok(
    document.querySelector("[data-testid='project-canvas-active-channels']"),
  );
  assert.ok(document.querySelector("[data-testid='project-canvas-reviews']"));
  assert.ok(document.querySelector("[data-testid='project-canvas-meetings']"));
  assert.deepEqual(JSON.parse(JSON.stringify(harness.sent.at(-1))), {
    dashboard: "dev",
    loadId: "load-1",
    nonce: "nonce-1",
    protocolVersion: 1,
    type: "canvas.rendered",
  });

  harness.port.emit(
    initMessage(harness.fixtureData, {
      project: { id: "project-1", name: "#my-home" },
    }),
  );
  assert.equal(
    document
      .querySelector("[data-testid='project-canvas-home-clock'] img")
      ?.getAttribute("src"),
    "buzz-canvas://localhost/template/package/assets/home-schedule-house.webp",
  );
  assert.equal(
    document
      .querySelector("[data-testid='project-canvas-home-schedule-gloopie']")
      ?.getAttribute("src"),
    "buzz-canvas://localhost/template/package/assets/gloopies-1.webm",
  );
  assert.ok(
    document.querySelector("[data-testid='project-canvas-home-clock']"),
  );
  assert.ok(
    document.querySelector("[data-testid='project-canvas-family-locations']"),
  );
  assert.ok(
    document.querySelector("[data-testid='project-canvas-chore-board']"),
  );
  const chore = document.querySelector(
    "[data-testid^='project-canvas-chore-'][type='checkbox']",
  );
  assert.ok(chore);
  chore.checked = true;
  chore.dispatchEvent(new harness.dom.window.Event("change"));
  harness.port.emit({
    loadId: "load-1",
    nonce: "nonce-1",
    protocolVersion: 1,
    snapshots: {
      channels: { data: [], status: "ready" },
      reviews: { data: [], status: "ready" },
    },
    type: "host.dataChanged",
  });
  assert.equal(chore.isConnected, true);
  assert.equal(chore.checked, true);

  harness.port.emit(
    initMessage(harness.fixtureData, {
      project: { id: "project-1", name: "my-support-channel" },
    }),
  );
  assert.ok(
    document.querySelector("[data-testid='project-canvas-release-notes']"),
  );
  assert.ok(
    document.querySelector("[data-testid='project-canvas-known-issues']"),
  );
  assert.ok(
    document.querySelector(
      "[data-testid='project-canvas-support-bug-reporter']",
    ),
  );
});

test("live queries drive the dev widgets through the SDK", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(initMessage(harness.fixtureData));

  const subscribes = harness.sent.filter(
    (message) => message.type === "canvas.subscribe",
  );
  const byQuery = Object.fromEntries(
    subscribes.map((message) => [message.query.name, message]),
  );
  assert.deepEqual(Object.keys(byQuery).sort(), [
    "project.channels.list",
    "project.reviews.list",
    "project.tasks.list",
  ]);
  for (const message of subscribes) {
    assert.equal(message.loadId, "load-1");
    assert.equal(message.nonce, "nonce-1");
    assert.equal(message.protocolVersion, 1);
  }
  const plain = (value) => JSON.parse(JSON.stringify(value));
  assert.deepEqual(plain(byQuery["project.reviews.list"].query.params), {
    status: "Open",
  });
  assert.deepEqual(plain(byQuery["project.tasks.list"].query.params), {
    limit: 8,
  });
  assert.equal(document.body.textContent.includes("Loading channels…"), true);

  const update = (subscriptionId, result) =>
    harness.port.emit({
      loadId: "load-1",
      nonce: "nonce-1",
      protocolVersion: 1,
      result,
      subscriptionId,
      type: "host.subscriptionUpdate",
    });

  update(byQuery["project.channels.list"].subscriptionId, {
    data: [],
    status: "ready",
  });
  assert.equal(document.body.textContent.includes("No channels to show"), true);

  update(byQuery["project.channels.list"].subscriptionId, {
    data: [
      {
        description: "Release candidate is ready",
        id: "channel-1",
        lastMessageAt: null,
        memberCount: 8,
        name: "real-release",
        people: [
          {
            avatarDataUrl: "data:image/png;base64,AA==",
            displayName: "Reviewer One",
            pubkey: "a".repeat(64),
          },
        ],
        relationship: "home",
        topic: null,
      },
    ],
    status: "ready",
  });
  const channelRow = document.querySelector(
    "[data-buzz-component='channel-row']",
  );
  assert.ok(channelRow);
  assert.equal(channelRow.tagName, "BUTTON");
  assert.equal(channelRow.textContent.includes("# real-release"), true);
  assert.equal(
    channelRow.textContent.includes("Release candidate is ready"),
    true,
  );
  assert.equal(
    channelRow
      .querySelector("[data-buzz-component='avatar'] img")
      ?.getAttribute("src"),
    `./__buzz/avatar/${"a".repeat(64)}`,
  );
  channelRow.click();
  const open = harness.sent.find((message) => message.type === "canvas.open");
  assert.ok(open);
  assert.deepEqual(plain(open.target), { id: "channel-1", type: "channel" });

  update(byQuery["project.reviews.list"].subscriptionId, {
    data: [
      {
        author: "a".repeat(64),
        branch: "feat/real-canvas",
        displayId: "1a2b3c4d",
        id: "1a2b3c4d".repeat(8),
        status: "Open",
        title: "Render actual review state",
        updatedAt: 1,
      },
    ],
    status: "ready",
  });
  assert.equal(
    document.body.textContent.includes("Render actual review state"),
    true,
  );
  assert.equal(document.body.textContent.includes("1 open"), true);
  const reviewRow = document.querySelector(
    "[data-buzz-component='review-row']",
  );
  assert.ok(reviewRow);
  assert.equal(
    reviewRow.querySelector(".buzz-status-pill")?.dataset.status,
    "open",
  );

  update(byQuery["project.tasks.list"].subscriptionId, {
    data: [
      {
        assignees: [],
        category: "Bug",
        commentCount: 2,
        displayId: "#42",
        id: "b".repeat(64),
        status: "Triage",
        title: "Fix the flaky test",
        updatedAt: 1,
      },
    ],
    status: "ready",
  });
  const buttons = () => [...document.querySelectorAll("button")];
  const markDone = buttons().find(
    (button) => button.textContent === "Mark done",
  );
  assert.ok(markDone);
  markDone.click();
  const command = harness.sent.find(
    (message) => message.type === "canvas.command",
  );
  assert.ok(command);
  assert.deepEqual(plain(command.command), {
    name: "tasks.setStatus",
    params: { id: "b".repeat(64), status: "done" },
  });
  assert.equal(markDone.disabled, true);
  harness.port.emit({
    commandId: command.commandId,
    loadId: "load-1",
    nonce: "nonce-1",
    protocolVersion: 1,
    result: { ok: true },
    type: "host.commandResult",
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(markDone.disabled, false);

  const assignToMe = buttons().find(
    (button) => button.textContent === "Assign to me",
  );
  assert.ok(assignToMe);
  assignToMe.click();
  const assign = harness.sent
    .filter((message) => message.type === "canvas.command")
    .at(-1);
  assert.deepEqual(plain(assign.command), {
    name: "tasks.assign",
    params: { id: "b".repeat(64) },
  });
});

test("the support bug reporter delivers the report as a DM to the project owner", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  const plain = (value) => JSON.parse(JSON.stringify(value));
  const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
  harness.port.emit(
    initMessage(harness.fixtureData, {
      project: { id: "project-1", name: "my-support-channel" },
    }),
  );

  const textarea = document.querySelector(
    "[data-testid='project-canvas-support-bug-input']",
  );
  const form = document.querySelector(
    "[data-testid='project-canvas-support-bug-reporter']",
  );
  assert.ok(textarea);
  assert.ok(form);
  textarea.value = "The sync button spins forever";
  textarea.dispatchEvent(new harness.dom.window.Event("input"));
  form.dispatchEvent(
    new harness.dom.window.Event("submit", { cancelable: true }),
  );

  const metadataQuery = harness.sent.find(
    (message) =>
      message.type === "canvas.query" &&
      message.query.name === "project.metadata",
  );
  assert.ok(metadataQuery);
  const owner = "c".repeat(64);
  harness.port.emit({
    loadId: "load-1",
    nonce: "nonce-1",
    protocolVersion: 1,
    queryId: metadataQuery.queryId,
    result: {
      data: { id: "30621:owner:proj", name: "proj", owner, repositories: [] },
      status: "ready",
    },
    type: "host.queryResult",
  });
  await tick();

  const dmCommand = harness.sent.find(
    (message) =>
      message.type === "canvas.command" && message.command.name === "dm.send",
  );
  assert.ok(dmCommand);
  assert.deepEqual(plain(dmCommand.command.params), {
    message: "Support report: The sync button spins forever",
    pubkey: owner,
  });
  harness.port.emit({
    commandId: dmCommand.commandId,
    loadId: "load-1",
    nonce: "nonce-1",
    protocolVersion: 1,
    result: { ok: true },
    type: "host.commandResult",
  });
  await tick();
  assert.equal(document.body.textContent.includes("Report sent"), true);
  assert.equal(
    document.body.textContent.includes(
      "Delivered to the project owner as a direct message.",
    ),
    true,
  );
});

test("the support bug reporter only stages the report without app.dm.send", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(
    initMessage(harness.fixtureData, {
      capabilities: ALL_CAPABILITIES.filter(
        (capability) => capability !== "app.dm.send",
      ),
      project: { id: "project-1", name: "my-support-channel" },
    }),
  );

  const textarea = document.querySelector(
    "[data-testid='project-canvas-support-bug-input']",
  );
  const form = document.querySelector(
    "[data-testid='project-canvas-support-bug-reporter']",
  );
  textarea.value = "The sync button spins forever";
  textarea.dispatchEvent(new harness.dom.window.Event("input"));
  form.dispatchEvent(
    new harness.dom.window.Event("submit", { cancelable: true }),
  );

  assert.equal(
    harness.sent.some(
      (message) =>
        message.type === "canvas.query" || message.type === "canvas.command",
    ),
    false,
  );
  assert.equal(document.body.textContent.includes("Report staged"), true);
});

test("missing capabilities render unavailable states and never subscribe", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(
    initMessage(harness.fixtureData, {
      capabilities: ["project.metadata.read"],
    }),
  );

  assert.equal(
    document.body.textContent.includes("Channels access unavailable"),
    true,
  );
  assert.equal(
    document.body.textContent.includes("Reviews access unavailable"),
    true,
  );
  assert.equal(
    document.body.textContent.includes("Tasks access unavailable"),
    true,
  );
  assert.equal(
    harness.sent.some((message) => message.type === "canvas.subscribe"),
    false,
  );
});

test("mode updates change package layout state without drawing a second fold marker", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(initMessage(harness.fixtureData));
  harness.port.emit({
    loadId: "load-1",
    mode: "full",
    nonce: "nonce-1",
    protocolVersion: 1,
    type: "host.mode",
  });
  assert.equal(
    document.querySelector("[data-testid='project-widget-canvas']")?.dataset
      .canvasMode,
    "full",
  );
  assert.equal(
    document.querySelectorAll("[data-testid='project-canvas-preview-boundary']")
      .length,
    0,
  );
});

test("targeted data updates preserve the widget root and receive previous and next state", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(
    initMessage(harness.fixtureData, {
      project: { id: "project-1", name: "my-home" },
    }),
  );
  const board = document.querySelector(
    "[data-testid='project-canvas-chore-board']",
  );
  const unrelated = document.querySelector(
    "[data-testid='project-canvas-home-clock']",
  );
  assert.ok(board);
  assert.ok(unrelated);

  const nextData = structuredClone(harness.fixtureData);
  const choreWidget = nextData.dashboards.home.widgets.find(
    (widget) => widget.id === "chores",
  );
  choreWidget.data.groups[1].completed = ["Take bins to the curb"];
  harness.port.emit({
    data: nextData,
    loadId: "load-1",
    nonce: "nonce-1",
    notificationId: "11111111111141118111111111111111",
    protocolVersion: 1,
    type: "host.widgetDataChanged",
    widgetId: "chores",
  });

  assert.equal(
    document.querySelector("[data-testid='project-canvas-chore-board']"),
    board,
  );
  assert.equal(
    document.querySelector("[data-testid='project-canvas-home-clock']"),
    unrelated,
  );
  assert.equal(board.dataset.previousCompleted, "1");
  assert.equal(board.dataset.completed, "2");
  assert.equal(board.classList.contains("widget-data-updated"), true);
  assert.equal(
    document.querySelector(
      "[data-testid='project-canvas-chore-jon-take-bins-to-the-curb']",
    )?.checked,
    true,
  );
});

const LAYOUT_FLUSH_MS = 400;

function flushLayoutSaves() {
  return new Promise((resolve) => setTimeout(resolve, LAYOUT_FLUSH_MS));
}

function layoutMessages(harness) {
  return harness.sent.filter((message) => message.type === "canvas.layout");
}

function widgetArticle(document, widgetId) {
  return document.querySelector(
    `[data-testid='project-canvas-widget-${widgetId}']`,
  );
}

test("persisted layout overrides seed widget positions and canvas pan", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(
    initMessage(harness.fixtureData, {
      layouts: {
        dev: {
          pan: { x: -24, y: 48 },
          sizes: { reviews: { height: 384, width: 528 } },
          widgets: { "active-channels": { x: 120, y: 96 } },
        },
      },
    }),
  );

  const moved = widgetArticle(document, "active-channels");
  assert.equal(moved.dataset.worldX, "120");
  assert.equal(moved.dataset.worldY, "96");
  assert.equal(
    moved.parentElement.style.transform,
    "translate3d(120px, 96px, 0)",
  );
  // A widget with no override keeps following the package default.
  assert.equal(widgetArticle(document, "reviews").dataset.worldX, "384");

  // Size overrides are independent of position overrides: the resized widget
  // keeps its package position, the moved widget keeps its package size.
  const resized = widgetArticle(document, "reviews");
  assert.equal(resized.dataset.worldWidth, "528");
  assert.equal(resized.dataset.worldHeight, "384");
  assert.equal(resized.parentElement.style.width, "528px");
  assert.equal(resized.parentElement.style.height, "384px");
  assert.equal(moved.dataset.worldWidth, "336");
  assert.equal(moved.parentElement.style.width, "336px");

  const canvas = document.querySelector(
    "[data-testid='project-widget-canvas']",
  );
  assert.equal(canvas.dataset.panX, "-24");
  assert.equal(canvas.dataset.panY, "48");
  assert.equal(
    canvas.querySelector(".canvas-world").style.transform,
    "translate3d(-24px, 48px, 0)",
  );
  assert.deepEqual(layoutMessages(harness), []);
});

test("keyboard nudges send one debounced layout carrying only the moved widget", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(initMessage(harness.fixtureData));

  const article = widgetArticle(document, "active-channels");
  for (const key of ["ArrowRight", "ArrowRight", "ArrowDown"]) {
    article.dispatchEvent(
      new harness.dom.window.KeyboardEvent("keydown", {
        cancelable: true,
        key,
      }),
    );
  }
  assert.equal(article.dataset.worldX, "48");
  assert.equal(article.dataset.worldY, "24");
  assert.deepEqual(layoutMessages(harness), [], "sends are debounced");

  await flushLayoutSaves();
  const saved = layoutMessages(harness);
  assert.equal(saved.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(saved[0])), {
    dashboard: "dev",
    loadId: "load-1",
    nonce: "nonce-1",
    pan: null,
    protocolVersion: 1,
    sizes: {},
    type: "canvas.layout",
    widgets: { "active-channels": { x: 48, y: 24 } },
  });
});

test("keyboard resizes send one debounced layout carrying only the resized widget", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(initMessage(harness.fixtureData));

  const handle = document.querySelector(
    "[data-testid='project-canvas-widget-active-channels-resize']",
  );
  assert.equal(
    handle.getAttribute("aria-label"),
    "Resize Active channels widget",
  );
  for (const key of ["ArrowRight", "ArrowRight", "ArrowDown"]) {
    handle.dispatchEvent(
      new harness.dom.window.KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        key,
      }),
    );
  }
  const article = widgetArticle(document, "active-channels");
  assert.equal(article.dataset.worldWidth, "384");
  assert.equal(article.dataset.worldHeight, "360");
  assert.equal(article.parentElement.style.width, "384px");
  assert.equal(article.parentElement.style.height, "360px");
  // Resizing must not create a position override for the widget.
  assert.equal(article.dataset.worldX, "0");
  assert.deepEqual(layoutMessages(harness), [], "sends are debounced");

  await flushLayoutSaves();
  const saved = layoutMessages(harness);
  assert.equal(saved.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(saved[0])), {
    dashboard: "dev",
    loadId: "load-1",
    nonce: "nonce-1",
    pan: null,
    protocolVersion: 1,
    sizes: { "active-channels": { height: 360, width: 384 } },
    type: "canvas.layout",
    widgets: {},
  });
});

test("keyboard resizes clamp at the minimum widget size", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(initMessage(harness.fixtureData));

  const handle = document.querySelector(
    "[data-testid='project-canvas-widget-active-channels-resize']",
  );
  // active-channels starts at 336x336; 8 shift-shrinks would go negative.
  for (let press = 0; press < 8; press += 1) {
    for (const key of ["ArrowLeft", "ArrowUp"]) {
      handle.dispatchEvent(
        new harness.dom.window.KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          key,
          shiftKey: true,
        }),
      );
    }
  }
  const article = widgetArticle(document, "active-channels");
  assert.equal(article.dataset.worldWidth, "192");
  assert.equal(article.dataset.worldHeight, "144");
});

test("resetting the canvas restores defaults and clears the stored overrides", async () => {
  const harness = await createCanvasHarness();
  const { document } = harness.dom.window;
  harness.port.emit(
    initMessage(harness.fixtureData, {
      layouts: {
        dev: {
          pan: { x: -24, y: 48 },
          sizes: { "active-channels": { height: 456, width: 480 } },
          widgets: { "active-channels": { x: 120, y: 96 } },
        },
      },
    }),
  );

  const reset = document.querySelector(
    "[data-testid='project-widget-canvas-reset']",
  );
  assert.equal(reset.getAttribute("aria-label"), "Reset canvas layout");
  reset.dispatchEvent(new harness.dom.window.Event("click"));

  const article = widgetArticle(document, "active-channels");
  assert.equal(article.dataset.worldX, "0");
  assert.equal(article.dataset.worldY, "0");
  assert.equal(
    article.parentElement.style.transform,
    "translate3d(0px, 0px, 0)",
  );
  assert.equal(article.dataset.worldWidth, "336");
  assert.equal(article.dataset.worldHeight, "336");
  assert.equal(article.parentElement.style.width, "336px");
  assert.equal(article.parentElement.style.height, "336px");
  const canvas = document.querySelector(
    "[data-testid='project-widget-canvas']",
  );
  assert.equal(canvas.dataset.panX, "24");
  assert.equal(canvas.dataset.panY, "24");

  await flushLayoutSaves();
  const saved = layoutMessages(harness);
  assert.equal(saved.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(saved[0].widgets)), {});
  assert.deepEqual(JSON.parse(JSON.stringify(saved[0].sizes)), {});
  assert.equal(saved[0].pan, null);
});
