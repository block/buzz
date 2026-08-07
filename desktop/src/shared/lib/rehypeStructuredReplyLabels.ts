/**
 * Rehype plugin that styles the fixed labels of a structured agent reply.
 *
 * Some agents answer in a fixed, labelled envelope — a status line, an answer
 * block (broken into "what it is / why it matters / done when"), a source, a
 * confidence, a next action, an owner, and a blocker/escalation line. This
 * plugin gives those labels the standard accent color and bold weight so the
 * structure reads at a glance. It is presentation only: it runs in the HAST
 * phase of the react-markdown pipeline (the same place as rehypeSearchHighlight),
 * so the stored message text is never altered — only wrapped for display.
 *
 * Safety by construction:
 *  - It acts ONLY when the message is a valid structured envelope: the seven
 *    top-level labels must appear as line-leading text, in the exact canonical
 *    order. Ordinary prose, other messages, and partial/out-of-order text are
 *    left completely untouched.
 *  - It styles ONLY the exact labels at structural line-leading positions
 *    (first text of a block, or the text immediately after a `<br>`). Values and
 *    body text are never styled.
 *  - It never descends into code, links, quotes, or pasted source
 *    (`code`/`pre`/`a`/`blockquote`), so an incidental "STATUS:" inside those is
 *    not styled.
 */

// Minimal HAST types — matches the pattern in rehypeSearchHighlight.ts.
interface HastText {
  type: "text";
  value: string;
}

interface HastElement {
  type: "element";
  tagName: string;
  properties: Record<string, unknown>;
  children: HastNode[];
}

type HastNode = HastElement | HastText | { type: string };

interface HastRoot {
  type: "root";
  children: HastNode[];
}

function isElement(node: HastNode): node is HastElement {
  return node.type === "element";
}

function isText(node: HastNode): node is HastText {
  return node.type === "text";
}

// The canonical structured-reply envelope (order is significant — it is the gate).
const TOP_LEVEL_LABELS = [
  "STATUS",
  "ANSWER",
  "SOURCE",
  "CONFIDENCE",
  "NEXT ACTION",
  "OWNER",
  "BLOCKER OR ESCALATION",
] as const;

// The three ANSWER sub-labels, also line-leading within the envelope.
const SUB_LABELS = ["What it is", "Why it matters", "Done when"] as const;

const ALL_LABELS: readonly string[] = [...TOP_LEVEL_LABELS, ...SUB_LABELS];

// Existing semantic accent (`--primary`, defined in both light and dark themes —
// the same token links use) plus the bold weight utility. No hard-coded color.
const LABEL_CLASS = "text-primary font-bold";

// Block containers whose first child begins a visual line. `blockquote` is
// block-level too but is handled as skip-only (see SKIP_TAGS).
const BLOCK_TAGS = new Set([
  "p",
  "div",
  "li",
  "td",
  "th",
  "dd",
  "dt",
  "section",
  "article",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
]);

// Never style a label inside these: code, pre, links, quotes/pasted source.
const SKIP_TAGS = new Set(["code", "pre", "a", "blockquote"]);

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Longest-first alternation so multi-word labels (e.g. "BLOCKER OR ESCALATION")
// win over any shorter prefix. Anchored at the start of a line-leading text
// node: up to three leading spaces (CommonMark strips more), the exact label,
// a colon, then end-of-text or whitespace so "STATUSes:" style near-misses and
// bare "STATUS" without a colon do not match.
const LABEL_AT_LINE_START = new RegExp(
  `^( {0,3})(${[...ALL_LABELS]
    .sort((a, b) => b.length - a.length)
    .map(escapeRegExp)
    .join("|")}):(?=$|\\s)`,
);

function matchTopLevel(value: string): string | null {
  const m = LABEL_AT_LINE_START.exec(value);
  return m && (TOP_LEVEL_LABELS as readonly string[]).includes(m[2])
    ? m[2]
    : null;
}

/** Split a line-leading text node into [leadingSpaces?, <span>label:</span>, rest?]. */
function styleLabel(node: HastText): HastNode[] {
  const m = LABEL_AT_LINE_START.exec(node.value);
  if (!m) return [node];
  const lead = m[1];
  const label = m[2];
  const rest = node.value.slice(m[0].length);
  const out: HastNode[] = [];
  if (lead) out.push({ type: "text", value: lead });
  out.push({
    type: "element",
    tagName: "span",
    properties: { className: LABEL_CLASS },
    children: [{ type: "text", value: `${label}:` }],
  });
  if (rest) out.push({ type: "text", value: rest });
  return out;
}

/**
 * Walk a children array tracking line-leading position. Collects every
 * line-leading text value into `leads`; when `style` is true it also rewrites
 * line-leading label text nodes into styled spans. `startAtLineStart` is true
 * for a block container's children and for an inline element that itself sits
 * at a line start.
 */
function processChildren(
  nodes: HastNode[],
  inSkip: boolean,
  style: boolean,
  leads: string[],
  startAtLineStart: boolean,
): HastNode[] {
  const result: HastNode[] = [];
  let atLineStart = startAtLineStart;

  for (const node of nodes) {
    if (isText(node)) {
      if (atLineStart && !inSkip) {
        leads.push(node.value);
        if (style) {
          result.push(...styleLabel(node));
          atLineStart = false;
          continue;
        }
      }
      result.push(node);
      atLineStart = false;
      continue;
    }

    if (isElement(node)) {
      if (node.tagName === "br") {
        result.push(node);
        atLineStart = true;
        continue;
      }
      const childSkip = inSkip || SKIP_TAGS.has(node.tagName);
      const isBlock =
        BLOCK_TAGS.has(node.tagName) || node.tagName === "blockquote";
      const newChildren = processChildren(
        node.children ?? [],
        childSkip,
        style,
        leads,
        isBlock ? true : atLineStart,
      );
      result.push(style ? { ...node, children: newChildren } : node);
      // A block element ends the current line; content after it begins a new
      // one. Inline content keeps the line going.
      atLineStart = isBlock;
      continue;
    }

    result.push(node);
    atLineStart = false;
  }

  return result;
}

export default function rehypeStructuredReplyLabels() {
  return (tree: HastRoot) => {
    // Pass 1: collect line-leading text and gate on the exact envelope.
    const leads: string[] = [];
    processChildren(tree.children, false, false, leads, true);
    const topLevelFound = leads
      .map(matchTopLevel)
      .filter((label): label is string => label !== null);
    const isEnvelope =
      topLevelFound.length === TOP_LEVEL_LABELS.length &&
      topLevelFound.every((label, i) => label === TOP_LEVEL_LABELS[i]);
    if (!isEnvelope) return;

    // Pass 2: style the labels in place.
    tree.children = processChildren(tree.children, false, true, [], true);
  };
}
