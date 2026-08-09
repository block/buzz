/**
 * Hard bounds on KaTeX rendering in the message timeline.
 *
 * KaTeX is synchronous pure computation — a hostile or malformed message
 * could otherwise push a long main-thread task onto the renderer. These
 * guards cap the blast radius before the element is ever cached:
 *
 * - A message with more than {@link MATH_LIMITS.maxFormulasPerMessage}
 *   formulas is pathological; math is disabled for the whole message and
 *   `$...$` renders literally.
 * - A single formula longer than {@link MATH_LIMITS.maxFormulaLength} is
 *   neutralised on its own (opening delimiter backslash-escaped) so the rest
 *   of the message still renders normally.
 *
 * Messages without any `$` short-circuit immediately (zero cost) — this is
 * the "no-op for non-math content" guarantee, mirroring remark-math's own
 * tokenizer-level short-circuit.
 */

export const MATH_LIMITS = {
  /** Hard cap on formulas per message before math is disabled entirely. */
  maxFormulasPerMessage: 50,
  /** Hard cap on a single formula's source length (in chars) before it is
   * neutralised to literal text. */
  maxFormulaLength: 2_000,
} as const;

const DISPLAY_OPEN = "$$";
const INLINE_OPEN = "$";

type Span = {
  start: number;
  end: number;
  /** true when an opening delimiter follows (display) vs single `$`. */
  display: boolean;
  oversized: boolean;
};

/**
 * Conservative scanner for `$...$` / `$$...$$` spans. Biased toward detection:
 * over-detecting a formula only degrades that one formula to literal text;
 * under-detecting could feed hostile math to KaTeX. Non-math content (`$5`,
 * currencies, unmatched `$`) does not produce spans.
 */
function findSpans(markdown: string): Span[] {
  const spans: Span[] = [];
  let i = 0;
  const n = markdown.length;
  while (i < n) {
    if (markdown.startsWith(DISPLAY_OPEN, i)) {
      // Display math: $$...$$, may span lines.
      const close = markdown.indexOf(DISPLAY_OPEN, i + 2);
      if (close !== -1) {
        const length = close - (i + 2);
        spans.push({
          start: i,
          end: close + 2,
          display: true,
          oversized: length > MATH_LIMITS.maxFormulaLength,
        });
        i = close + 2;
        continue;
      }
      i += 1; // unmatched "$$" -> literal
      continue;
    }
    if (markdown[i] === INLINE_OPEN && markdown[i + 1] !== INLINE_OPEN) {
      // Inline math: $...$, single line, content may not contain a nested `$`
      // and must be non-empty (mirrors remark-math's "tight" rule).
      const nextNewline = markdown.indexOf("\n", i + 1);
      const nextDollar = markdown.indexOf(INLINE_OPEN, i + 1);
      if (
        nextDollar !== -1 &&
        nextDollar !== i + 1 &&
        (nextNewline === -1 || nextDollar < nextNewline)
      ) {
        const length = nextDollar - (i + 1);
        spans.push({
          start: i,
          end: nextDollar + 1,
          display: false,
          oversized: length > MATH_LIMITS.maxFormulaLength,
        });
        i = nextDollar + 1;
        continue;
      }
      i += 1; // lone/loose `$` -> literal
      continue;
    }
    i += 1;
  }
  return spans;
}

export type MathBounds = {
  /** Markdown to feed to react-markdown (may differ from the input only when
   * an oversized formula has been neutralised). */
  content: string;
  /** When true, remark-math + rehype-katex must be omitted entirely. */
  disableMath: boolean;
};

export function applyMathBounds(markdown: string): MathBounds {
  if (!markdown.includes(INLINE_OPEN)) {
    return { content: markdown, disableMath: false };
  }
  const spans = findSpans(markdown);
  if (spans.length === 0) {
    return { content: markdown, disableMath: false };
  }
  if (spans.length > MATH_LIMITS.maxFormulasPerMessage) {
    return { content: markdown, disableMath: true };
  }
  const anyOversized = spans.some((span) => span.oversized);
  if (!anyOversized) {
    return { content: markdown, disableMath: false };
  }
  // Neutralise oversized formulas so remark-math does not tokenise them and
  // they render as literal text, while the well-formed formulas in the same
  // message keep rendering.
  let out = markdown;
  let shift = 0;
  for (const span of spans) {
    if (!span.oversized) continue;
    if (span.display) {
      // Display `$$...$$` is finicky: remark-math still reads a `$$` that has
      // only one preceding backslash, and a lone *unescaped* trailing `$$` is
      // itself tokenised back into that same display span. So a single escape
      // of the opener is not enough. Neutralise by breaking BOTH delimiters
      // into `\$ \$` (backslash-dollar backslash-dollar) -- with no contiguous
      // `$$` left anywhere, remark-math has nothing to latch onto and the
      // whole span renders literally (no katex, no red error).
      const openAt = span.start + shift;
      out = out.slice(0, openAt) + "\\$\\$" + out.slice(openAt + 2);
      shift += 2;
      const closeAt = span.end + shift - 2;
      out = out.slice(0, closeAt) + "\\$\\$" + out.slice(closeAt + 2);
      shift += 2;
    } else {
      // Inline `$...$`: escape the opening delimiter. (Escaping the closing
      // `$` too would yield `$...\$`, which KaTeX mis-reads and red-highlights
      // -- so we leave the closing lone `$` alone; a lone `$` never starts
      // math.) Insert at the span's current (post-insertion) offset.
      const at = span.start + shift;
      out = out.slice(0, at) + "\\" + out.slice(at);
      shift += 1;
    }
  }
  return { content: out, disableMath: false };
}
