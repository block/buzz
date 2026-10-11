import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { BrowserScreenshotViewer } from "./BrowserScreenshotViewer.tsx";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let root;
let host;
afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
});

function mount(over = {}) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  const calls = { close: 0, news: 0 };
  const props = { busy: false, disabled: false, onClose: () => calls.close++, onNew: () => calls.news++, src: "data:image/jpeg;base64,AAAA", statusText: null, statusTone: "info", takenAt: 1_700_000_000, title: "Docs", url: "https://docs.example.com/", ...over };
  act(() => root.render(React.createElement(BrowserScreenshotViewer, props)));
  return calls;
}
const q = (id) => document.querySelector(`[data-testid="${id}"]`);

test("shows title, url, taken time and the large image", () => {
  mount();
  assert.equal(q("shot-viewer-title").textContent, "Docs");
  assert.equal(q("shot-viewer-url").textContent, "https://docs.example.com/");
  assert.match(q("shot-viewer-time").textContent, /^Screenshot taken /);
  assert.equal(q("shot-viewer-img").getAttribute("src"), "data:image/jpeg;base64,AAAA");
});

test("Close button, Esc and tapping outside close; tapping inside does not", () => {
  const calls = mount();
  act(() => q("shot-viewer-panel").click());
  assert.equal(calls.close, 0);
  act(() => q("shot-viewer-close").click());
  assert.equal(calls.close, 1);
  act(() => q("shot-viewer").click());
  assert.equal(calls.close, 2);
  act(() => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
  assert.equal(calls.close, 3);
});

test("New screenshot calls back, is disabled while busy, and shows status; empty state without an image", () => {
  const calls = mount({ src: null, takenAt: null, statusText: "Screenshot x: sent, waiting for your desktop…" });
  assert.match(q("shot-viewer-time").textContent, /No screenshot yet/);
  assert.equal(q("shot-viewer-img"), null);
  assert.match(q("shot-viewer-status").textContent, /waiting for your desktop/);
  act(() => q("shot-viewer-new").click());
  assert.equal(calls.news, 1);
  act(() => root.unmount());
  host.remove();
  mount({ busy: true });
  assert.equal(q("shot-viewer-new").hasAttribute("disabled"), true);
});
