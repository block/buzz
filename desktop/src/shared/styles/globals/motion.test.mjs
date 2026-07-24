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
