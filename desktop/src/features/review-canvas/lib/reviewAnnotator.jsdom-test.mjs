import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { JSDOM } from "jsdom";

import {
  ANNOTATOR_INIT_TYPE,
  buildInitMessage,
  parseAnnotatorMessage,
} from "./reviewAnnotatorProtocol.ts";
import { buildFrameDocument, parseReviewDocument } from "./reviewDocument.ts";
import { THREE_BLOCK_HTML } from "./reviewFixtures.mjs";

// The shipped static asset, evaluated exactly as the frame would run it.
const ANNOTATOR_SOURCE = readFileSync(
  new URL("../../../../public/synaxis-review-annotator.js", import.meta.url),
  "utf8",
);

const PRIMARY = "checkout.primary-action";

function fakePort() {
  const sent = [];
  return {
    sent,
    onmessage: null,
    // A real MessagePort structured-clones: the parent never shares the
    // frame's object identities or prototypes.
    postMessage(message) {
      sent.push(JSON.parse(JSON.stringify(message)));
    },
    /** Deliver a message from trusted chrome to the annotator. */
    deliver(data) {
      this.onmessage?.({ data });
    },
  };
}

/** Compose the real frame document, run the real annotator, hand over a port. */
function openFrame({ html = THREE_BLOCK_HTML, handshake = true } = {}) {
  const document = parseReviewDocument(html);
  const srcdoc = buildFrameDocument(document, {
    nonce: "AAAAAAAAAAAAAAAAAAAAAAAA",
    annotatorSrc: "https://app.example/synaxis-review-annotator.js",
  });
  const dom = new JSDOM(srcdoc, { runScripts: "outside-only" });
  const { window } = dom;
  window.eval(ANNOTATOR_SOURCE);
  const port = fakePort();
  const deliverInit = (overrides = {}, target = port) => {
    const event = new window.Event("message");
    Object.defineProperty(event, "source", {
      value: "source" in overrides ? overrides.source : window.parent,
    });
    Object.defineProperty(event, "data", {
      value: overrides.data ?? buildInitMessage(document.blocks),
    });
    Object.defineProperty(event, "ports", { value: [target] });
    window.dispatchEvent(event);
  };
  if (handshake) deliverInit();
  const block = (id) =>
    window.document.querySelector(`[data-synaxis-review-id="${id}"]`);
  const key = (target, init) => {
    const event = new window.KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      ...init,
    });
    target.dispatchEvent(event);
    return event;
  };
  const click = (target) => {
    const event = new window.MouseEvent("click", {
      bubbles: true,
      cancelable: true,
    });
    target.dispatchEvent(event);
    return event;
  };
  const selections = () =>
    port.sent.filter((message) => message.type === "select");
  return { window, port, block, key, click, selections, deliverInit, document };
}

test("the annotator makes blocks focusable without replacing native semantics", () => {
  const { port, block } = openFrame();
  assert.deepEqual(port.sent, [
    {
      type: "ready",
      blockIds: ["checkout.header", PRIMARY, "checkout.summary"],
    },
  ]);
  const primary = block(PRIMARY);
  assert.equal(primary.getAttribute("tabindex"), "0");
  assert.equal(primary.localName, "section");
  assert.equal(primary.getAttribute("role"), null);
  assert.match(primary.getAttribute("aria-label"), /Primary checkout action/);
});

test("Enter on a focused block selects it", () => {
  const { block, key, selections } = openFrame();
  const primary = block(PRIMARY);
  primary.focus();
  const event = key(primary, { key: "Enter" });
  assert.deepEqual(selections(), [{ type: "select", id: PRIMARY }]);
  assert.equal(event.defaultPrevented, true);
});

test("Space on a focused block selects it without scrolling the page", () => {
  const { block, key, selections } = openFrame();
  const summary = block("checkout.summary");
  summary.focus();
  const event = key(summary, { key: " " });
  assert.deepEqual(selections(), [{ type: "select", id: "checkout.summary" }]);
  assert.equal(event.defaultPrevented, true);
});

test("modified, repeated, and unrelated keys never select (every modality is distinct)", () => {
  const { block, key, selections } = openFrame();
  const primary = block(PRIMARY);
  primary.focus();
  key(primary, { key: " ", shiftKey: true });
  key(primary, { key: "Enter", ctrlKey: true });
  key(primary, { key: "Enter", metaKey: true });
  key(primary, { key: "Enter", altKey: true });
  key(primary, { key: "Enter", repeat: true });
  key(primary, { key: "a" });
  key(primary, { key: "Tab" });
  assert.deepEqual(selections(), []);
});

test("keys typed into a control inside a block belong to that control", () => {
  const { block, key, selections } = openFrame();
  const button = block(PRIMARY).querySelector("button");
  button.focus();
  const event = key(button, { key: "Enter" });
  assert.deepEqual(selections(), []);
  assert.equal(event.defaultPrevented, false);
});

test("pointer click anywhere inside a block selects the innermost declared block", () => {
  const { block, click, selections } = openFrame();
  const event = click(block(PRIMARY).querySelector("button"));
  assert.deepEqual(selections(), [{ type: "select", id: PRIMARY }]);
  assert.equal(event.defaultPrevented, true);
});

test("clicks outside any declared block select nothing, and links never navigate", () => {
  const { window, click, selections } = openFrame({
    html: `<!doctype html><body><p id="outside">plain</p><section data-synaxis-review-id="only" data-synaxis-review-title="Only"><a id="link">x</a></section></body>`,
  });
  click(window.document.getElementById("outside"));
  assert.deepEqual(selections(), []);
  // The sanitizer strips href, but the annotator is the second line of defence.
  const link = window.document.getElementById("link");
  link.setAttribute("href", "https://evil.example/");
  assert.equal(click(link).defaultPrevented, true);
});

test("selection and focus messages from chrome update the frame", () => {
  const { window, port, block } = openFrame();
  port.deliver({ type: "selection", id: PRIMARY });
  assert.equal(block(PRIMARY).getAttribute("data-synaxis-selected"), "true");
  assert.equal(block(PRIMARY).getAttribute("aria-current"), "true");
  assert.equal(
    block("checkout.summary").hasAttribute("data-synaxis-selected"),
    false,
  );

  port.deliver({ type: "selection", id: "checkout.summary" });
  assert.equal(block(PRIMARY).hasAttribute("data-synaxis-selected"), false);
  assert.equal(
    block("checkout.summary").getAttribute("data-synaxis-selected"),
    "true",
  );

  port.deliver({ type: "selection", id: null });
  assert.equal(
    block("checkout.summary").hasAttribute("data-synaxis-selected"),
    false,
  );

  port.deliver({ type: "focus", id: PRIMARY });
  assert.equal(window.document.activeElement, block(PRIMARY));
  // Unknown ids and junk are ignored.
  port.deliver({ type: "focus", id: "nope" });
  port.deliver("junk");
  port.deliver(null);
  assert.equal(window.document.activeElement, block(PRIMARY));
});

test("hover and focus highlights are part of the composed frame stylesheet", () => {
  const { window } = openFrame();
  const css = [...window.document.querySelectorAll("style")]
    .map((style) => style.textContent)
    .join("\n");
  assert.match(css, /\[data-synaxis-review-id\]:hover\s*\{[^}]*outline/);
  assert.match(css, /:focus-visible/);
  assert.match(css, /data-synaxis-selected="true"/);
});

test("the capability is one-time: a second init is ignored", () => {
  const { port, deliverInit, block, key, selections } = openFrame();
  const intruder = fakePort();
  deliverInit({}, intruder);
  assert.deepEqual(intruder.sent, [], "the second port never receives ready");
  block(PRIMARY).focus();
  key(block(PRIMARY), { key: "Enter" });
  assert.equal(selections().length, 1);
  assert.equal(port.sent.filter((m) => m.type === "select").length, 1);
});

test("init from anything but the embedding window, or with a malformed body, is ignored", () => {
  for (const overrides of [
    { source: {} },
    { source: null },
    { data: { type: "other", blocks: [] } },
    { data: { type: ANNOTATOR_INIT_TYPE, blocks: "nope" } },
    { data: { type: ANNOTATOR_INIT_TYPE, blocks: [{ id: 1, title: "x" }] } },
    {
      data: {
        type: ANNOTATOR_INIT_TYPE,
        blocks: Array.from({ length: 201 }, (_, i) => ({
          id: `b${i}`,
          title: "t",
        })),
      },
    },
  ]) {
    const frame = openFrame({ handshake: false });
    frame.deliverInit(overrides, frame.port);
    assert.deepEqual(
      frame.port.sent,
      [],
      JSON.stringify(overrides.data ?? "source"),
    );
  }
});

test("ids the parent did not declare are not exposed even if the markup carries them", () => {
  const { window, port } = openFrame({ handshake: false });
  const section = window.document.createElement("section");
  section.setAttribute("data-synaxis-review-id", "smuggled");
  window.document.body.append(section);
  const event = new window.Event("message");
  Object.defineProperty(event, "source", { value: window.parent });
  Object.defineProperty(event, "data", {
    value: {
      type: ANNOTATOR_INIT_TYPE,
      blocks: [{ id: "checkout.summary", title: "Order summary" }],
    },
  });
  Object.defineProperty(event, "ports", { value: [port] });
  window.dispatchEvent(event);
  assert.deepEqual(port.sent, [
    { type: "ready", blockIds: ["checkout.summary"] },
  ]);
  assert.equal(section.hasAttribute("tabindex"), false);
});

// ── Parent-side validation of what the frame says ───────────────────────────

test("the parent honours only select and ready messages naming blocks it declared", () => {
  const known = new Set(["a", "b"]);
  assert.deepEqual(parseAnnotatorMessage({ type: "select", id: "a" }, known), {
    type: "select",
    id: "a",
  });
  assert.deepEqual(
    parseAnnotatorMessage({ type: "ready", blockIds: ["a", "b"] }, known),
    { type: "ready", blockIds: ["a", "b"] },
  );
  for (const hostile of [
    { type: "select", id: "c" },
    { type: "select", id: 7 },
    { type: "select" },
    { type: "ready", blockIds: ["a", "evil"] },
    { type: "ready", blockIds: ["a", "a", "b"] },
    { type: "ready", blockIds: "a" },
    { type: "submit", id: "a" },
    { type: "select", id: ["a"] },
    "select",
    null,
    undefined,
  ]) {
    assert.equal(
      parseAnnotatorMessage(hostile, known),
      null,
      JSON.stringify(hostile),
    );
  }
});
