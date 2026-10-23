import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const motionCss = readFileSync(
  new URL("./motion.css", import.meta.url),
  "utf8",
);

test("conversation arrival uses shared motion tokens", () => {
  assert.match(motionCss, /--motion-duration-arrival:\s*500ms/);
  assert.match(motionCss, /--motion-ease-arrival:/);
  assert.match(
    motionCss,
    /\.motion-enter-conversation\s*\{[\s\S]*var\(--motion-duration-arrival\)[\s\S]*var\(--motion-ease-arrival\)/,
  );
});

test("conversation arrival has a reduced-motion treatment", () => {
  assert.match(
    motionCss,
    /@media \(prefers-reduced-motion: reduce\)[\s\S]*\.motion-enter-conversation/,
  );
});

test("send entrance uses shared tokens and avoids transforms and filters", () => {
  const rule = motionCss.match(/\.motion-enter-send\s*\{[^}]*\}/)?.[0] ?? "";
  assert.match(rule, /var\(--motion-duration-standard\)/);
  assert.match(rule, /var\(--motion-ease-standard\)/);

  // Transforms or filters inside virtualized timeline rows leave stale paint
  // in WKWebView, so the send entrance animates only track size and opacity.
  for (const name of ["motion-enter-send-open", "motion-enter-send-fade"]) {
    const keyframes =
      motionCss.match(
        new RegExp(`@keyframes ${name}\\s*\\{[\\s\\S]*?\\n\\}`),
      )?.[0] ?? "";
    assert.ok(keyframes, `missing @keyframes ${name}`);
    assert.doesNotMatch(keyframes, /transform|filter/);
  }
});

test("send entrance has a reduced-motion treatment", () => {
  assert.match(
    motionCss,
    /@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.motion-enter-send\s*\{\s*animation:\s*none/,
  );
});
