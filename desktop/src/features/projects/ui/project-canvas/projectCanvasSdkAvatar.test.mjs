import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

// Avatars reach a sandboxed frame over `__buzz/avatar/<pubkey>` rather than as
// base64 inside an RPC message. These bind the frame half of that: which URL
// the SDK asks for, and what a person looks like before and after it answers.

const testDirectory = path.dirname(fileURLToPath(import.meta.url));
const desktopRoot = existsSync(
  path.resolve(process.cwd(), "desktop/package.json"),
)
  ? path.resolve(process.cwd(), "desktop")
  : path.resolve(testDirectory, "../../../../..");
const require = createRequire(
  pathToFileURL(path.join(desktopRoot, "package.json")),
);
const { JSDOM } = require("jsdom");

const PUBKEY = "ab".repeat(32);
const DATA_URL = "data:image/webp;base64,QUJD";

/** Loads the host-owned SDK into a frame-shaped document and returns its `ui`. */
async function loadSdkUi() {
  const dom = new JSDOM('<main id="canvas-root"></main>', {
    runScripts: "outside-only",
    url: "buzz-canvas://localhost/0123456789abcdef/",
  });
  Object.defineProperty(dom.window, "buzzCanvas", {
    configurable: false,
    value: Object.freeze({
      packageBaseUrl: "buzz-canvas://localhost/0123456789abcdef/package/",
      port: {
        addEventListener() {},
        postMessage() {},
        start() {},
      },
      protocolVersion: 1,
      sdk: {},
    }),
    writable: false,
  });
  dom.window.eval(
    await readFile(
      path.join(desktopRoot, "src-tauri/src/project_canvas_package/sdk.js"),
      "utf8",
    ),
  );
  return { dom, ui: dom.window.buzzCanvas.sdk.ui };
}

function image(node) {
  return node.querySelector("img.buzz-avatar-image");
}

function fallback(node) {
  return node.querySelector(".buzz-avatar-fallback");
}

test("a pubkey resolves to the host avatar route, not an inlined payload", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ name: "Ada Lovelace", pubkey: PUBKEY });

  // Relative so it resolves against the frame's own load id; `base-uri 'none'`
  // in the canvas CSP is what stops a package repointing it.
  assert.equal(image(node).getAttribute("src"), `./__buzz/avatar/${PUBKEY}`);
  assert.equal(
    new URL(image(node).src).pathname,
    `/0123456789abcdef/__buzz/avatar/${PUBKEY}`,
  );
});

test("an uppercase pubkey is normalized into the route", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ name: "Ada", pubkey: PUBKEY.toUpperCase() });

  assert.equal(image(node).getAttribute("src"), `./__buzz/avatar/${PUBKEY}`);
});

test("initials show until the picture loads and survive a 404", async () => {
  const { dom, ui } = await loadSdkUi();

  const node = ui.avatar({ name: "Ada Lovelace", pubkey: PUBKEY });

  // Present from the first paint: the route answers 404 for anyone the host
  // has not published, and that is ordinary rather than an error.
  assert.equal(fallback(node).textContent, "AL");
  assert.ok(image(node));

  image(node).dispatchEvent(new dom.window.Event("error"));

  assert.equal(image(node), null);
  assert.equal(fallback(node).textContent, "AL");
});

test("a person with no pubkey and no data url renders initials alone", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ name: "Grace Hopper" });

  assert.equal(image(node), null);
  assert.equal(fallback(node).textContent, "GH");
  assert.equal(node.dataset.tone, String(node.dataset.tone));
});

test("a malformed pubkey falls back rather than requesting a bad route", async () => {
  const { ui } = await loadSdkUi();

  for (const pubkey of ["", "beef", `${PUBKEY}00`, "zz".repeat(32)]) {
    const node = ui.avatar({ name: "Ada", pubkey });
    assert.equal(image(node), null, `pubkey ${pubkey || "(empty)"}`);
  }
});

test("an inlined data url still renders for widgets written before the route", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ avatarUrl: DATA_URL, name: "Ada" });

  assert.equal(image(node).getAttribute("src"), DATA_URL);
});

test("a pubkey wins over an inlined data url", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ avatarUrl: DATA_URL, name: "Ada", pubkey: PUBKEY });

  assert.equal(image(node).getAttribute("src"), `./__buzz/avatar/${PUBKEY}`);
});

test("a non-data avatarUrl is refused so the frame cannot be made to fetch", async () => {
  const { ui } = await loadSdkUi();

  // `connect-src 'none'` blocks XHR but not images; refusing the URL outright
  // keeps a hostile widget from turning an avatar into a tracking beacon.
  for (const avatarUrl of [
    "https://example.test/pixel.png",
    "//example.test/pixel.png",
    "javascript:alert(1)",
  ]) {
    const node = ui.avatar({ avatarUrl, name: "Ada" });
    assert.equal(image(node), null, avatarUrl);
  }
});

test("the avatar is one labelled image to a screen reader", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ name: "Ada Lovelace", pubkey: PUBKEY });

  // One owner for the label: the wrapper is the image, and the inner <img>
  // stays decorative so it is not a second stop.
  assert.equal(node.getAttribute("role"), "img");
  assert.equal(node.getAttribute("aria-label"), "Ada Lovelace");
  assert.equal(image(node).getAttribute("alt"), "");
});

test("an unnamed person is still labelled", async () => {
  const { ui } = await loadSdkUi();

  const node = ui.avatar({ pubkey: PUBKEY });

  assert.equal(node.getAttribute("aria-label"), "Unknown person");
  assert.equal(fallback(node).textContent, "?");
});

test("channel rows carry each person's pubkey through to the route", async () => {
  const { ui } = await loadSdkUi();

  const row = ui.channelRow({
    channel: {
      id: "channel-1",
      name: "general",
      people: [{ displayName: "Ada Lovelace", pubkey: PUBKEY }],
    },
  });

  // Binds the reason a widget gets real faces without changing: the row
  // passes the pubkey it already has, so every member can show a picture
  // rather than only the handful the RPC budget could inline.
  assert.equal(
    row.querySelector("img.buzz-avatar-image").getAttribute("src"),
    `./__buzz/avatar/${PUBKEY}`,
  );
});
