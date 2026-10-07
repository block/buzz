import {
  ReviewContractError,
  type ReviewBlock,
  validateReviewBlock,
} from "./reviewContract";

/** Marker attributes a factory uses to declare a reviewable region. */
export const REVIEW_ID_ATTRIBUTE = "data-synaxis-review-id";
const REVIEW_TITLE_ATTRIBUTE = "data-synaxis-review-title";
const REVIEW_SOURCE_REF_ATTRIBUTE = "data-synaxis-source-ref";
const MAX_REVIEW_BLOCKS = 200;

/**
 * Elements removed outright. Active content (scripts, frames, plugins, forms,
 * media that fetches), document-level redirects (`meta`, `base`, `link`), and
 * markup whose parse differs when scripting is enabled (`noscript`,
 * `template`) or that smuggles foreign namespaces (`math`, `foreignObject`).
 * This is a second line of defence behind the frame's CSP and sandbox.
 */
const REMOVED_ELEMENTS = new Set([
  "script",
  "iframe",
  "frame",
  "frameset",
  "object",
  "embed",
  "applet",
  "form",
  "base",
  "meta",
  "link",
  "noscript",
  "template",
  "portal",
  "audio",
  "video",
  "source",
  "track",
  "canvas",
  "math",
  "foreignobject",
  "set",
  "animate",
  "animatemotion",
  "animatetransform",
]);

/** Attributes that navigate, fetch, submit, or make content editable/focusable. */
const REMOVED_ATTRIBUTES = new Set([
  "href",
  "action",
  "formaction",
  "srcdoc",
  "srcset",
  "ping",
  "poster",
  "background",
  "manifest",
  "data",
  "codebase",
  "target",
  "contenteditable",
  "accesskey",
  "autofocus",
  "tabindex",
  "nonce",
  "integrity",
  "http-equiv",
]);

const DATA_IMAGE_SOURCE =
  /^data:image\/(?:png|jpe?g|gif|webp|avif|svg\+xml)[;,]/i;

/** Passes the scrub may take before its output is judged unstable and dropped. */
const MAX_CSS_SCRUB_PASSES = 8;

function scrubCssOnce(css: string): string {
  return (
    css
      // A legacy `<!--` wrapper is meaningless to CSS and is the only `<` a
      // stylesheet legitimately carries; removing it keeps the rule after it.
      .replace(/<!--/g, "")
      .replace(/@import[^;{]*;?/gi, "")
      .replace(/url\(\s*(?!["']?data:)[^)]*\)/gi, "url()")
      .replace(/(?:-webkit-)?image-set\s*\((?:[^()]|\([^()]*\))*\)/gi, "none")
      .replace(/expression\s*\((?:[^()]|\([^()]*\))*\)/gi, "none")
      .replace(/expression\s*\(/gi, "blocked(")
      .replace(/javascript\s*:/gi, "blocked:")
      .replace(/-moz-binding/gi, "blocked")
  );
}

/**
 * Make CSS safe to place inside an HTML raw-text element (`<style>`): every
 * literal `<` becomes the CSS escape `\3c ` (the trailing space ends the hex
 * escape). A raw-text element only ends at `</style`, so with no `<` left no
 * text, however it was produced, can close the element or start a tag.
 * Idempotent: the output contains no `<`.
 */
export function escapeCssForRawText(css: string): string {
  return css.replace(/</g, "\\3c ");
}

/**
 * Strip network references and script hooks from CSS text, and make the result
 * safe to embed in a `<style>` element.
 *
 * Deleting a match can splice its neighbours into a new match (`@imp@import;ort`
 * becomes `@import`) or into `</style`, which was never present in the parsed
 * (inert) source. So the removals run to a fixpoint, and only then is `<`
 * escaped: the returned text can never contain a closing tag. A stylesheet
 * that is still changing after a bounded number of passes is dropped whole.
 */
export function scrubCss(css: string): string {
  let current = css;
  for (let pass = 0; pass < MAX_CSS_SCRUB_PASSES; pass += 1) {
    const next = scrubCssOnce(current);
    if (next === current) return escapeCssForRawText(current);
    current = next;
  }
  return "";
}

const SAME_DOCUMENT_FRAGMENT = /^#[A-Za-z0-9_.:-]+$/;

/** Whether an attribute is active content (or CSS that still needs scrubbing). */
function isActiveAttribute(tag: string, attribute: Attr): boolean {
  const name = attribute.localName.toLowerCase();
  // SVG icon sprites reference a same-document symbol; nothing else may link.
  if (
    tag === "use" &&
    name === "href" &&
    SAME_DOCUMENT_FRAGMENT.test(attribute.value)
  ) {
    return false;
  }
  if (name.startsWith("on") || REMOVED_ATTRIBUTES.has(name)) return true;
  if (name === "src") return !DATA_IMAGE_SOURCE.test(attribute.value.trim());
  if (name === "style") return scrubCss(attribute.value) !== attribute.value;
  return false;
}

function scrubAttributes(element: Element) {
  const tag = element.localName.toLowerCase();
  for (const attribute of Array.from(element.attributes)) {
    if (!isActiveAttribute(tag, attribute)) continue;
    if (attribute.localName.toLowerCase() === "style") {
      element.setAttribute(attribute.name, scrubCss(attribute.value));
    } else {
      element.removeAttribute(attribute.name);
    }
  }
}

/**
 * Replace a `<form>` with a `<div>` carrying the same attributes and children.
 * A mock-up's layout often lives inside a form (and a block marker may sit on
 * it), so dropping the element would blank reviewable regions; unwrapping
 * removes only the submission behaviour.
 */
function unwrapForm(form: Element): Element {
  const replacement = form.ownerDocument.createElement("div");
  for (const attribute of Array.from(form.attributes)) {
    try {
      replacement.setAttribute(attribute.name, attribute.value);
    } catch {
      // Attribute names the DOM API rejects cannot be re-emitted safely.
    }
  }
  while (form.firstChild) replacement.appendChild(form.firstChild);
  form.replaceWith(replacement);
  return replacement;
}

/**
 * Remove active content from a parsed (inert) document in place. The document
 * must come from `DOMParser`, which neither runs scripts nor loads resources.
 */
export function sanitizeReviewDocument(doc: Document) {
  // SHOW_COMMENT: comments are a classic mutation-XSS carrier across reparse.
  const comments = doc.createTreeWalker(doc, 128);
  const commentNodes: Node[] = [];
  while (comments.nextNode()) commentNodes.push(comments.currentNode);
  for (const comment of commentNodes) comment.parentNode?.removeChild(comment);

  for (const original of Array.from(doc.querySelectorAll("*"))) {
    let element = original;
    const tag = element.localName.toLowerCase();
    if (tag === "form") {
      element = unwrapForm(element);
    } else if (REMOVED_ELEMENTS.has(tag)) {
      element.remove();
      continue;
    }
    scrubAttributes(element);
    if (tag === "style") {
      element.textContent = scrubCss(element.textContent ?? "");
    }
  }
}

export type ReviewDocument = {
  /** Declared blocks in document order. */
  blocks: ReviewBlock[];
  /** Scrubbed `<style>` sources found in the document head. */
  headStyles: string[];
  /** Sanitized body markup. */
  bodyHtml: string;
  bodyClass: string | null;
  bodyStyle: string | null;
  lang: string | null;
  dir: "ltr" | "rtl" | "auto" | null;
};

function extractBlocks(root: Element): ReviewBlock[] {
  const elements = Array.from(
    root.querySelectorAll(`[${REVIEW_ID_ATTRIBUTE}]`),
  );
  if (elements.length > MAX_REVIEW_BLOCKS) {
    throw new ReviewContractError(
      `The document declares more than ${MAX_REVIEW_BLOCKS} review blocks.`,
    );
  }
  const seen = new Set<string>();
  return elements.map((element) => {
    const block = validateReviewBlock({
      id: element.getAttribute(REVIEW_ID_ATTRIBUTE),
      title: element.getAttribute(REVIEW_TITLE_ATTRIBUTE),
      sourceRef: element.getAttribute(REVIEW_SOURCE_REF_ATTRIBUTE),
    });
    if (seen.has(block.id)) {
      throw new ReviewContractError(
        `Review block ID "${block.id}" is declared more than once.`,
      );
    }
    seen.add(block.id);
    return block;
  });
}

/** Stand-ins used only to audit a parsed document's composed frame. */
const PROBE_NONCE = "AAAAAAAAAAAAAAAAAAAAAAAA";
const PROBE_ANNOTATOR_SRC = "https://probe.invalid/review-annotator.js";

/**
 * Parse, sanitize, and extract the declared review blocks from untrusted
 * factory HTML. Any marker that violates the contract rejects the document
 * (fail closed): a half-honoured review surface would let feedback land on a
 * region the factory did not declare. The result is also composed into a probe
 * frame and audited, so a document whose sanitized text would not survive
 * serialization is refused here, with the document, rather than at render.
 */
export function parseReviewDocument(html: string): ReviewDocument {
  const doc = new DOMParser().parseFromString(html, "text/html");
  sanitizeReviewDocument(doc);
  const body = doc.body;
  const lang = doc.documentElement.getAttribute("lang");
  const dir = doc.documentElement.getAttribute("dir");
  const parsed: ReviewDocument = {
    blocks: extractBlocks(body),
    headStyles: Array.from(doc.head.querySelectorAll("style")).map(
      (style) => style.textContent ?? "",
    ),
    bodyHtml: body.innerHTML,
    bodyClass: body.getAttribute("class"),
    bodyStyle: body.getAttribute("style"),
    lang: lang && /^[A-Za-z0-9-]{1,35}$/.test(lang) ? lang : null,
    dir: dir === "ltr" || dir === "rtl" || dir === "auto" ? dir : null,
  };
  buildFrameDocument(parsed, {
    nonce: PROBE_NONCE,
    annotatorSrc: PROBE_ANNOTATOR_SRC,
  });
  return parsed;
}

/**
 * The CSP the frame document carries. Only the one nonce-bearing annotator
 * script may run; there is no network, frame, form, base, or plugin surface.
 * Inline styles and data-URI images/fonts are the only resources permitted.
 */
export function buildFrameCsp(nonce: string): string {
  return [
    "default-src 'none'",
    `script-src 'nonce-${nonce}'`,
    "style-src 'unsafe-inline'",
    "img-src data:",
    "font-src data:",
    "media-src 'none'",
    "connect-src 'none'",
    "frame-src 'none'",
    "child-src 'none'",
    "object-src 'none'",
    "worker-src 'none'",
    "manifest-src 'none'",
    "form-action 'none'",
    "base-uri 'none'",
  ].join("; ");
}

/** Ink and halo of the block indicator; see {@link buildFrameChromeCss}. */
const CHROME_INK = "#0f172a";
const CHROME_HALO = "#ffffff";

/**
 * Hover, keyboard-focus, and selection indicators for declared blocks. Trusted
 * chrome draws them, so artifact CSS must not be able to hide or dilute them:
 *
 * - Every declaration is `!important` inside a cascade layer that precedes all
 *   artifact styles (the chrome `<style>` is emitted first). For `!important`
 *   declarations an earlier layer beats every later and every unlayered one
 *   whatever the selector specificity, so artifact stylesheets cannot override
 *   it. The layer name carries the per-document nonce, so artifact CSS cannot
 *   add rules to the layer either. The one author declaration no stylesheet
 *   outranks is an `!important` in a block's own inline `style` attribute.
 * - Selectors anchor at `html body` and repeat the marker attribute, which
 *   also wins on specificity should layers ever be unavailable.
 * - The indicator is two layers: a dark outline over a white `box-shadow`
 *   halo that fills the offset gap and a band outside the outline. Dark clears
 *   3:1 on backgrounds at or above ~13% relative luminance and white on those
 *   at or below ~30%, so on any artifact background (light, dark, or blue) at
 *   least one layer does. Forced-colors modes drop `box-shadow` but keep the
 *   outline.
 *
 * Hover is dashed, focus solid, and selection solid and thicker, so state is
 * never carried by colour alone.
 */
function buildFrameChromeCss(nonce: string): string {
  const marker = `[${REVIEW_ID_ATTRIBUTE}]`;
  const block = `html body ${marker}${marker}${marker}`;
  return `
@layer synaxis-review-chrome-${nonce} {
  ${block} { cursor: pointer !important; }
  ${block}:hover {
    outline: 2px dashed ${CHROME_INK} !important;
    outline-offset: 2px !important;
    box-shadow: 0 0 0 6px ${CHROME_HALO} !important;
  }
  ${block}:focus,
  ${block}:focus-visible {
    outline: 3px solid ${CHROME_INK} !important;
    outline-offset: 2px !important;
    box-shadow: 0 0 0 7px ${CHROME_HALO} !important;
  }
  ${block}[data-synaxis-selected="true"] {
    outline: 4px solid ${CHROME_INK} !important;
    outline-offset: 2px !important;
    box-shadow: 0 0 0 8px ${CHROME_HALO} !important;
  }
}
`;
}

function escapeAttribute(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function optionalAttribute(name: string, value: string | null): string {
  return value === null ? "" : ` ${name}="${escapeAttribute(value)}"`;
}

const UNSAFE_FRAME_MESSAGE =
  "The review document could not be composed safely, so it was not rendered.";

/**
 * Re-parse the composed frame exactly as the browser will and fail closed
 * unless it is what composition intended. Sanitizing the artifact's parse tree
 * proves nothing about what the *serialized* text parses to: text transformed
 * after the first parse (CSS above all) can end a raw-text element and turn
 * the rest of itself into live markup. So the frame is judged on its reparse:
 *
 * - the head is exactly the trusted CSP, charset, and chrome stylesheet plus
 *   the artifact's head stylesheets, each holding exactly its escaped text, so
 *   no stylesheet closed early and no `meta` refresh, `base`, `link`, or
 *   script materialized;
 * - the one script is the annotator, last in the body;
 * - nothing else is an active element or attribute (the sanitizer's own
 *   rules); and
 * - the declared blocks are exactly the blocks the reviewer is shown.
 */
function assertFrameIsInert(
  srcdoc: string,
  document: ReviewDocument,
  nonce: string,
) {
  const fail = (): never => {
    throw new ReviewContractError(UNSAFE_FRAME_MESSAGE);
  };
  const doc = new DOMParser().parseFromString(srcdoc, "text/html");
  const { body } = doc;
  for (const attribute of Array.from(doc.documentElement.attributes)) {
    if (attribute.name !== "lang" && attribute.name !== "dir") fail();
  }
  for (const attribute of Array.from(body.attributes)) {
    if (attribute.name !== "class" && attribute.name !== "style") fail();
  }

  const head = Array.from(doc.head.children);
  const [csp, charset, chrome, ...styles] = head;
  if (
    head.length !== 3 + document.headStyles.length ||
    csp?.localName !== "meta" ||
    csp.attributes.length !== 2 ||
    csp.getAttribute("http-equiv") !== "Content-Security-Policy" ||
    csp.getAttribute("content") !== buildFrameCsp(nonce) ||
    charset?.localName !== "meta" ||
    charset.attributes.length !== 1 ||
    charset.getAttribute("charset") !== "utf-8" ||
    chrome?.localName !== "style" ||
    chrome.attributes.length !== 1 ||
    !chrome.hasAttribute("data-synaxis-review-chrome") ||
    chrome.textContent !== buildFrameChromeCss(nonce) ||
    styles.some(
      (style, index) =>
        style.localName !== "style" ||
        style.attributes.length !== 0 ||
        style.textContent !== escapeCssForRawText(document.headStyles[index]),
    )
  ) {
    fail();
  }

  const scripts = doc.querySelectorAll("script");
  const annotator = scripts[0];
  // The nonce attribute is not read back: browsers hide it from script.
  if (
    scripts.length !== 1 ||
    annotator.parentElement !== body ||
    body.lastElementChild !== annotator ||
    annotator.attributes.length !== 3 ||
    !annotator.hasAttribute("nonce") ||
    !annotator.hasAttribute("data-synaxis-review-annotator") ||
    annotator.childNodes.length !== 0
  ) {
    fail();
  }

  const trusted = new Set<Element>([...head, annotator]);
  for (const element of Array.from(doc.querySelectorAll("*"))) {
    if (trusted.has(element)) continue;
    const tag = element.localName.toLowerCase();
    if (REMOVED_ELEMENTS.has(tag)) fail();
    for (const attribute of Array.from(element.attributes)) {
      if (isActiveAttribute(tag, attribute)) fail();
    }
  }

  const declared = Array.from(
    body.querySelectorAll(`[${REVIEW_ID_ATTRIBUTE}]`),
    (element) => element.getAttribute(REVIEW_ID_ATTRIBUTE) ?? "",
  ).sort();
  const expected = document.blocks.map((block) => block.id).sort();
  if (
    declared.length !== expected.length ||
    declared.some((id, index) => id !== expected[index])
  ) {
    fail();
  }
}

/**
 * Compose the isolated frame document. `annotatorSrc` is the one external
 * script (the review annotator, a static first-party asset); `nonce` must be
 * fresh per document so no artifact markup can predict it.
 *
 * Artifact CSS is escaped again at this sink (a no-op for text the sanitizer
 * produced) and the finished text is re-parsed and audited before it is
 * returned; anything unexpected throws `ReviewContractError` and nothing is
 * rendered.
 */
export function buildFrameDocument(
  document: ReviewDocument,
  options: { nonce: string; annotatorSrc: string },
): string {
  if (!/^[A-Za-z0-9_-]{16,}$/.test(options.nonce)) {
    throw new ReviewContractError("The frame nonce is malformed.");
  }
  const annotatorUrl = new URL(options.annotatorSrc);
  const srcdoc = [
    "<!doctype html>",
    `<html${optionalAttribute("lang", document.lang)}${optionalAttribute("dir", document.dir)}>`,
    "<head>",
    `<meta http-equiv="Content-Security-Policy" content="${escapeAttribute(buildFrameCsp(options.nonce))}">`,
    '<meta charset="utf-8">',
    `<style data-synaxis-review-chrome>${buildFrameChromeCss(options.nonce)}</style>`,
    ...document.headStyles.map(
      (css) => `<style>${escapeCssForRawText(css)}</style>`,
    ),
    "</head>",
    `<body${optionalAttribute("class", document.bodyClass)}${optionalAttribute("style", document.bodyStyle)}>`,
    document.bodyHtml,
    `<script nonce="${options.nonce}" src="${escapeAttribute(annotatorUrl.href)}" data-synaxis-review-annotator></script>`,
    "</body>",
    "</html>",
  ].join("\n");
  assertFrameIsInert(srcdoc, document, options.nonce);
  return srcdoc;
}

/** A fresh, unguessable CSP nonce for one composed frame document. */
export function createFrameNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(18));
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}
