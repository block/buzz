import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { pinWebviewLabelForWindow } from "../lib/pinWebview.ts";
import { resolvePinStartUrl } from "./PinnedSiteScreen.tsx";

const PINS = [
  { id: "openclaw-1111", url: "https://openclaw.example.com/" },
  { id: "github-2222", url: "https://github.com/huladesk" },
  { id: "wayfinder", url: "https://wayfinder.huladesk.com/" },
  { id: "porthole-3333", url: "https://porthole.example.com/app" },
];

test("switching pins never hands one pin's URL to another pin's webview", () => {
  // Walk every ordered pair, including back-and-forth, carrying the stale
  // state exactly as the screen does on the first render after a switch.
  const order = [0, 1, 0, 2, 1, 3, 0, 3, 2, 0];
  let state = { pinId: PINS[order[0]].id, url: PINS[order[0]].url };
  for (const index of order.slice(1)) {
    const pin = PINS[index];
    assert.equal(resolvePinStartUrl(state, pin), pin.url, pin.id);
    state = { pinId: pin.id, url: pin.url };
  }
});

test("a pin keeps its own deep link (state) when it is the same pin", () => {
  const pin = PINS[0];
  const deep = "https://openclaw.example.com/repo/main";
  assert.equal(resolvePinStartUrl({ pinId: pin.id, url: deep }, pin), deep);
  assert.equal(resolvePinStartUrl({ pinId: pin.id, url: "" }, pin), pin.url);
  assert.equal(resolvePinStartUrl({ pinId: "x", url: deep }, null), "");
});

test("each pin's native webview is keyed by its stable id, not position", () => {
  const labels = PINS.map((pin) => pinWebviewLabelForWindow(pin.id));
  assert.equal(new Set(labels).size, PINS.length);
  PINS.forEach((pin, i) => assert.equal(labels[i], `pin-${pin.id}`));
  // Reordering pins does not change any label.
  const reversed = [...PINS].reverse().map((pin) => pinWebviewLabelForWindow(pin.id));
  assert.deepEqual([...reversed].reverse(), labels);
});

test("pin route remounts the screen per pin id", () => {
  const dir = dirname(fileURLToPath(import.meta.url));
  const route = readFileSync(
    join(dir, "../../../app/routes/pins.$pinId.tsx"),
    "utf8",
  );
  assert.match(route, /<PinnedSiteScreen key=\{pinId\} pinId=\{pinId\} \/>/);
});
