import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isPdfPageInRenderWindow,
  isRelayMediaHref,
  pdfPageAtScroll,
  pdfPageCounterLabel,
  pdfRenderScale,
} from "./pdfPages.ts";

const SHA = "a".repeat(64);
const RELAY = "https://relay.example";

test("isRelayMediaHref: relay /media/ URL routes through the native fetch", () => {
  assert.equal(isRelayMediaHref(`${RELAY}/media/${SHA}.pdf`, RELAY), true);
});

test("isRelayMediaHref: external Blossom host is fetched directly", () => {
  assert.equal(
    isRelayMediaHref(`https://nostr.build/media/${SHA}.pdf`, RELAY),
    false,
  );
});

test("isRelayMediaHref: host comparison is case-insensitive", () => {
  assert.equal(
    isRelayMediaHref(`https://RELAY.example/media/${SHA}.pdf`, RELAY),
    true,
  );
});

test("isRelayMediaHref: unresolved relay origin defaults to the relay path", () => {
  assert.equal(isRelayMediaHref(`${RELAY}/media/${SHA}.pdf`, null), true);
});

test("isRelayMediaHref: non-media paths and garbage are not relay media", () => {
  assert.equal(isRelayMediaHref(`${RELAY}/files/${SHA}.pdf`, RELAY), false);
  assert.equal(isRelayMediaHref("not a url", null), false);
});

test("pdfRenderScale: fits the page to the CSS width at the device ratio", () => {
  assert.equal(pdfRenderScale(612, 306, 1), 0.5);
  assert.equal(pdfRenderScale(612, 306, 2), 1);
});

test("pdfRenderScale: caps the device pixel ratio at 2x", () => {
  assert.equal(pdfRenderScale(600, 600, 3), 2);
});

test("pdfRenderScale: treats missing or sub-1 ratios as 1x", () => {
  assert.equal(pdfRenderScale(600, 600, 0), 1);
  assert.equal(pdfRenderScale(600, 600, 0.5), 1);
});

test("pdfRenderScale: degenerate sizes fall back to scale 1", () => {
  assert.equal(pdfRenderScale(0, 400, 2), 1);
  assert.equal(pdfRenderScale(612, 0, 2), 1);
});

test("pdfPageAtScroll: first page at the top of the document", () => {
  assert.equal(pdfPageAtScroll([0, 800, 1600], 0, 600), 1);
});

test("pdfPageAtScroll: page advances once its top crosses the upper third", () => {
  // probe = scrollTop + 200
  assert.equal(pdfPageAtScroll([0, 800, 1600], 599, 600), 1);
  assert.equal(pdfPageAtScroll([0, 800, 1600], 600, 600), 2);
  assert.equal(pdfPageAtScroll([0, 800, 1600], 1500, 600), 3);
});

test("pdfPageAtScroll: empty documents report page 1", () => {
  assert.equal(pdfPageAtScroll([], 500, 600), 1);
});

test("isPdfPageInRenderWindow: only pages near the current page render", () => {
  assert.equal(isPdfPageInRenderWindow(5, 5, 2), true);
  assert.equal(isPdfPageInRenderWindow(3, 5, 2), true);
  assert.equal(isPdfPageInRenderWindow(7, 5, 2), true);
  assert.equal(isPdfPageInRenderWindow(2, 5, 2), false);
  assert.equal(isPdfPageInRenderWindow(8, 5, 2), false);
});

test("pdfPageCounterLabel: formats and clamps the page counter", () => {
  assert.equal(pdfPageCounterLabel(3, 17), "3 / 17");
  assert.equal(pdfPageCounterLabel(0, 17), "1 / 17");
  assert.equal(pdfPageCounterLabel(20, 17), "17 / 17");
});
