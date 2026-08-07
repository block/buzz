import assert from "node:assert/strict";
import test from "node:test";

import rehypeStructuredReplyLabels from "./rehypeStructuredReplyLabels.ts";

const LABEL_CLASS = "text-primary font-bold";

// --- tiny HAST builders (mirror what remark-rehype + remark-breaks emit) ---
const t = (value) => ({ type: "text", value });
const br = () => ({
  type: "element",
  tagName: "br",
  properties: {},
  children: [],
});
const el = (tagName, children, properties = {}) => ({
  type: "element",
  tagName,
  properties,
  children,
});
const root = (children) => ({ type: "root", children });

/** One paragraph whose lines are separated by <br> (the remark-breaks shape). */
function paragraphWithBreaks(lines) {
  const kids = [];
  lines.forEach((line, i) => {
    if (i > 0) kids.push(br());
    kids.push(typeof line === "string" ? t(line) : line);
  });
  return el("p", kids);
}

function apply(tree) {
  rehypeStructuredReplyLabels()(tree);
  return tree;
}

function visibleText(node) {
  if (node.type === "text") return node.value;
  if (Array.isArray(node.children))
    return node.children.map(visibleText).join("");
  return "";
}

function styledLabels(node, acc = []) {
  if (
    node.type === "element" &&
    node.tagName === "span" &&
    node.properties?.className === LABEL_CLASS
  ) {
    acc.push(visibleText(node));
  }
  for (const child of node.children ?? []) styledLabels(child, acc);
  return acc;
}

const ENVELOPE_LINES = [
  "STATUS: [FACT] Live",
  "ANSWER:",
  "What it is: Example Agent is a demo assistant.",
  "Why it matters: It shows the reply structure.",
  "Done when: The reply is delivered.",
  "SOURCE: Example source v1.0",
  "CONFIDENCE: high",
  "NEXT ACTION:",
  "1. Review the reply.",
  "OWNER: Operator",
  "BLOCKER OR ESCALATION: No blocker.",
];

const ALL_TEN = [
  "STATUS:",
  "ANSWER:",
  "What it is:",
  "Why it matters:",
  "Done when:",
  "SOURCE:",
  "CONFIDENCE:",
  "NEXT ACTION:",
  "OWNER:",
  "BLOCKER OR ESCALATION:",
];

test("styles all ten labels in a valid envelope (br-joined paragraph)", () => {
  const tree = root([paragraphWithBreaks(ENVELOPE_LINES)]);
  apply(tree);
  assert.deepEqual(styledLabels(tree), ALL_TEN);
});

test("styles all ten labels when each line is its own block (paragraph shape)", () => {
  const tree = root(ENVELOPE_LINES.map((line) => el("p", [t(line)])));
  apply(tree);
  assert.deepEqual(styledLabels(tree).sort(), [...ALL_TEN].sort());
});

test("values and body text are never styled", () => {
  const tree = root([paragraphWithBreaks(ENVELOPE_LINES)]);
  apply(tree);
  for (const label of styledLabels(tree)) assert.ok(ALL_TEN.includes(label));
  assert.equal(styledLabels(tree).length, 10);
});

test("raw visible text is unchanged (renderer-only, byte-for-byte)", () => {
  const tree = root([paragraphWithBreaks(ENVELOPE_LINES)]);
  const before = visibleText(tree);
  apply(tree);
  assert.equal(visibleText(tree), before);
});

test("numbered NEXT ACTION item is not styled (only the label)", () => {
  const tree = root([paragraphWithBreaks(ENVELOPE_LINES)]);
  apply(tree);
  assert.ok(!styledLabels(tree).some((s) => s.includes("Review the reply")));
  assert.ok(styledLabels(tree).includes("NEXT ACTION:"));
});

test("false positive: prose containing a lone 'STATUS:' is untouched", () => {
  const tree = root([el("p", [t("STATUS: everything is on track today")])]);
  const clone = structuredClone(tree);
  apply(tree);
  assert.equal(styledLabels(tree).length, 0);
  assert.deepEqual(tree, clone);
});

test("false positive: label not at line-leading position is untouched", () => {
  const tree = root([el("p", [t("please check the STATUS: soon")])]);
  apply(tree);
  assert.equal(styledLabels(tree).length, 0);
});

test("false positive: out-of-order labels are not an envelope", () => {
  const scrambled = [
    "STATUS: [FACT] Live",
    "ANSWER:",
    "What it is: x",
    "Why it matters: y",
    "Done when: z",
    "OWNER: Operator", // OWNER before SOURCE/CONFIDENCE/NEXT ACTION
    "CONFIDENCE: high",
    "SOURCE: Example source",
    "NEXT ACTION: none",
    "BLOCKER OR ESCALATION: none",
  ];
  const tree = root([paragraphWithBreaks(scrambled)]);
  apply(tree);
  assert.equal(styledLabels(tree).length, 0);
});

test("false positive: partial envelope (missing labels) is untouched", () => {
  const tree = root([
    paragraphWithBreaks(["STATUS: Live", "ANSWER:", "OWNER: Operator"]),
  ]);
  apply(tree);
  assert.equal(styledLabels(tree).length, 0);
});

test("a label inside inline code within an envelope is not styled", () => {
  const lines = ENVELOPE_LINES.map((line) =>
    line.startsWith("CONFIDENCE:")
      ? [t("CONFIDENCE: "), el("code", [t("STATUS: x")])]
      : line,
  );
  const kids = [];
  lines.forEach((line, i) => {
    if (i > 0) kids.push(br());
    if (Array.isArray(line)) kids.push(...line);
    else kids.push(t(line));
  });
  const tree = root([el("p", kids)]);
  apply(tree);
  const labels = styledLabels(tree);
  assert.equal(labels.length, 10); // exactly the ten line-leading labels
  assert.ok(labels.includes("CONFIDENCE:"));
  assert.equal(labels.filter((l) => l === "STATUS:").length, 1);
});

test("an envelope quoted inside a blockquote is not styled", () => {
  const tree = root([el("blockquote", [paragraphWithBreaks(ENVELOPE_LINES)])]);
  const clone = structuredClone(tree);
  apply(tree);
  assert.equal(styledLabels(tree).length, 0);
  assert.deepEqual(tree, clone);
});

test("ordinary markdown (bold, list, link) is left byte-for-byte unchanged", () => {
  const tree = root([
    el("p", [t("Here is "), el("strong", [t("bold")]), t(" text.")]),
    el("ul", [el("li", [t("first")]), el("li", [t("second")])]),
    el("p", [el("a", [t("a link")], { href: "https://example.com" })]),
  ]);
  const clone = structuredClone(tree);
  apply(tree);
  assert.equal(styledLabels(tree).length, 0);
  assert.deepEqual(tree, clone);
});
