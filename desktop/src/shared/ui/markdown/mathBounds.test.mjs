import assert from "node:assert/strict";
import test from "node:test";

import { MATH_LIMITS, applyMathBounds } from "./mathBounds.ts";

// Pure guardrail tests for the KaTeX maths-bounds scanner. These cover the
// Canvas performance red-lines: non-math content is a no-op, oversized or
// excessive formulas are neutralised before reaching KaTeX.

test("content without a dollar sign is a zero-cost no-op", () => {
  const src = "**bold** and `code` with no math";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(content, src);
  assert.equal(disableMath, false);
});

test("currency / plain dollar text is not treated as math", () => {
  // remark-math rejects `$5 and $10` (content ends on whitespace); our
  // scanner may over-detect but must never disable math for a benign message
  // nor mutate it.
  const src = "The price is $5 and $10 today.";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(content, src);
  assert.equal(disableMath, false);
});

test("well-formed inline math is kept as-is", () => {
  const src = "Energy $E=mc^2$ is famous.";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(content, src);
  assert.equal(disableMath, false);
});

test("well-formed display math is kept as-is", () => {
  const src = "$$\n\\int_{-\\infty}^{\\infty} e^{-x^2} dx = \\sqrt{\\pi}\n$$";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(content, src);
  assert.equal(disableMath, false);
});

test("a message with too many formulas disables math entirely", () => {
  const count = MATH_LIMITS.maxFormulasPerMessage + 1;
  const src = Array.from({ length: count }, (_, i) => `$x_{${i}}$`).join(" ");
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, true);
  assert.equal(content, src, "math is disabled, content unchanged");
});

test("an oversized inline formula is neutralised on its own", () => {
  const huge = "a".repeat(MATH_LIMITS.maxFormulaLength + 1);
  const src = `boom $${huge}$ end`;
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, false, "math stays enabled for the rest");
  assert.ok(
    content.startsWith("boom \\$"),
    `opening inline delimiter escaped: ${content.slice(0, 12)}`,
  );
  assert.ok(content.endsWith("$ end"));
});

test("an oversized display formula is neutralised on its own", () => {
  const huge = "b".repeat(MATH_LIMITS.maxFormulaLength + 1);
  const src = `before $$${huge}$$ after`;
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, false);
  assert.ok(
    content.startsWith("before \\$\\$"),
    "opening display delimiter escaped",
  );
  assert.ok(content.endsWith("\\$\\$ after"));
  assert.ok(!/\\$\\$/.test(content), "no contiguous $$ left to tokenise");
});

test("only the oversized formula is neutralised; siblings untouched", () => {
  const huge = "c".repeat(MATH_LIMITS.maxFormulaLength + 1);
  const src = `ok $y=1$ then $${huge}$ done`;
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, false);
  // The healthy `$y=1$` is untouched.
  assert.ok(content.includes("$y=1$"), "healthy sibling formula unchanged");
  // The oversized one has its opening `$` escaped.
  assert.ok(
    content.includes("then \\$") && content.endsWith("$ done"),
    "only the oversized opening delimiter is escaped",
  );
});
