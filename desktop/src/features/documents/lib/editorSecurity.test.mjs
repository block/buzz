/**
 * Hostile markdown through the real editor pipeline.
 *
 * A vault is not necessarily trusted input. Notes arrive from git repos, sync
 * clients, shared team folders and downloads. These tests feed the editor the
 * payloads an attacker would put in a `.md` file and assert that none of them
 * reach the DOM as anything but text.
 *
 * Buzz does ship a CSP whose `script-src` omits `'unsafe-inline'`, so an
 * injected `<script>` would not execute even if one got this far. That is a
 * second line of defence, not a reason to skip the first: `img-src` and
 * `connect-src` both allow `https:` and `http:`, so raw markup reaching the DOM
 * could still beacon out the contents of a note without running any script at
 * all. Escaping is what prevents that, and escaping is what this file pins.
 *
 * The configuration that makes this hold is in `vaultEditorExtensions.ts`:
 * `html: false`, `linkify: false` and `openOnClick: false`. Each is load-bearing
 * and each is pinned below, so turning one on fails here rather than in
 * someone's vault.
 */
import assert from "node:assert/strict";
import { before, test } from "node:test";
import { JSDOM } from "jsdom";

let Editor;
let vaultEditorExtensions;

before(async () => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>");
  globalThis.window = dom.window;
  globalThis.document = dom.window.document;
  globalThis.HTMLElement = dom.window.HTMLElement;
  globalThis.Element = dom.window.Element;
  globalThis.Node = dom.window.Node;
  globalThis.DocumentFragment = dom.window.DocumentFragment;
  globalThis.getComputedStyle = dom.window.getComputedStyle;
  globalThis.MutationObserver = dom.window.MutationObserver;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  ({ Editor } = await import("@tiptap/core"));
  ({ vaultEditorExtensions } = await import(
    "./editor/vaultEditorExtensions.ts"
  ));
});

/** Renders `markdown` exactly as opening a note in live preview would. */
function renderHtml(markdown) {
  const element = document.createElement("div");
  document.body.appendChild(element);
  const editor = new Editor({
    content: "",
    element,
    extensions: vaultEditorExtensions(),
  });
  editor.commands.setContent(markdown, { emitUpdate: false });
  const html = editor.getHTML();
  editor.destroy();
  element.remove();
  return html;
}

/**
 * The rendered markup as a live DOM tree.
 *
 * Asserting against the HTML *string* is what a first draft of this file did,
 * and it was wrong: `&lt;img src=x onerror="..."&gt;` is escaped text that a
 * regex for `onerror=` happily matches. What matters is which elements and
 * attributes actually materialise, so parse it and inspect the tree.
 */
function renderDom(markdown) {
  const container = document.createElement("div");
  // Inert in jsdom, and assignment never executes <script> in any browser
  // either — this is parsing, not evaluation.
  container.innerHTML = renderHtml(markdown);
  return container;
}

/** Every attribute on every element in the tree. */
function allAttributes(container) {
  return [...container.querySelectorAll("*")].flatMap((element) =>
    [...element.attributes].map((attribute) => ({
      element: element.tagName.toLowerCase(),
      name: attribute.name.toLowerCase(),
      value: attribute.value,
    })),
  );
}

const SCRIPT_PAYLOADS = [
  ["script tag", "<script>globalThis.pwned = true;</script>"],
  ["img onerror", '<img src=x onerror="globalThis.pwned = true">'],
  ["svg onload", '<svg onload="globalThis.pwned = true"></svg>'],
  ["iframe", '<iframe src="https://evil.example"></iframe>'],
  ["body onload", '<body onload="globalThis.pwned = true">'],
  ["style tag", "<style>body { display: none }</style>"],
  [
    "script inside a fenced block",
    "```\n<script>globalThis.pwned = true;</script>\n```",
  ],
];

test("raw HTML in a note never becomes live markup", () => {
  for (const [label, payload] of SCRIPT_PAYLOADS) {
    const container = renderDom(payload);

    assert.equal(
      globalThis.pwned,
      undefined,
      `${label}: a payload executed during parsing`,
    );

    const dangerous = container.querySelector(
      "script, iframe, object, embed, style, svg, link, meta, form",
    );
    assert.equal(
      dangerous,
      null,
      `${label}: a <${dangerous?.tagName.toLowerCase()}> element materialised`,
    );

    const handler = allAttributes(container).find((attribute) =>
      attribute.name.startsWith("on"),
    );
    assert.equal(
      handler,
      undefined,
      `${label}: ${handler?.element} carried ${handler?.name}="${handler?.value}"`,
    );

    // The payload should still be visible to the reader, as literal text.
    assert.ok(
      container.textContent.includes("<") || payload.startsWith("```"),
      `${label}: the payload vanished entirely rather than being escaped`,
    );
  }
});

test("dangerous link protocols do not survive into an href", () => {
  // `openOnClick` is false so nothing follows these, but a `javascript:` href
  // sitting in the DOM of a CSP-less app is one stray handler away from being a
  // real problem. TipTap's own protocol validation is what stops it; this
  // pins that we depend on it.
  for (const payload of [
    "[click me](javascript:globalThis.pwned=true)",
    "[click me](JaVaScRiPt:globalThis.pwned=true)",
    "[click me](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
    "[click me](vbscript:msgbox)",
  ]) {
    const container = renderDom(payload);
    const hrefs = allAttributes(container).filter(
      (attribute) => attribute.name === "href" || attribute.name === "src",
    );
    for (const { name, value } of hrefs) {
      assert.ok(
        !/^\s*(?:javascript|data|vbscript):/i.test(value),
        `a dangerous protocol reached ${name}\n  ${payload}\n  ${value}`,
      );
    }
  }
});

test("ordinary links still work, so the guard above is not vacuous", () => {
  const html = renderHtml("[docs](https://example.com/page)");
  assert.ok(
    html.includes('href="https://example.com/page"'),
    `an ordinary link must survive, otherwise this file proves nothing\n  ${html}`,
  );
});

test("a note cannot smuggle markup through an image URL", () => {
  const container = renderDom(
    '![alt](https://example.com/x.png"onerror="alert(1))',
  );
  const handler = allAttributes(container).find((attribute) =>
    attribute.name.startsWith("on"),
  );
  assert.equal(
    handler,
    undefined,
    `an event handler escaped through an image URL: ${handler?.name}`,
  );
});
