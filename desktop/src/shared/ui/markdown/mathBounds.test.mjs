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

test("currency / plain dollar text is neutralised to literal", () => {
  // remark-math (micromark-extension-math@3) accepts `$5 and $10` as math —
  // content may contain spaces — so the guard must actively escape the opener
  // to keep currency literal. The rendered output shows `$5 and $10` as text.
  const src = "The price is $5 and $10 today.";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, false);
  assert.equal(content, "The price is \\$5 and $10 today.");
});

test("CJK currency text is neutralised to literal", () => {
  const src = "价格在 $5 到 $10 之间";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, false);
  assert.equal(content, "价格在 \\$5 到 $10 之间");
});

test("loose inline span with whitespace inside a delimiter is neutralised", () => {
  const { content, disableMath } = applyMathBounds("loose $ x$ here");
  assert.equal(disableMath, false);
  assert.equal(content, "loose \\$ x$ here");
});

test("genuine math starting with a digit still renders", () => {
  // The digit rule guards the *closing* delimiter only (`$5 and $10`), so
  // real formulas like `$2^x$` are untouched.
  const src = "see $2^x$ done";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(content, src);
  assert.equal(disableMath, false);
});

test("a neutralised currency opener does not swallow a later real formula", () => {
  // After escaping `$5 ...`, scanning must resume at the would-be closing `$`
  // so the genuine `$x=1$` right after still counts as math.
  const src = "pay $5 and $x=1$ end";
  const { content, disableMath } = applyMathBounds(src);
  assert.equal(disableMath, false);
  assert.equal(content, "pay \\$5 and $x=1$ end");
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
  assert.ok(
    !content.includes("$$"),
    "no contiguous $$ remains to tokenise as display math",
  );
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
