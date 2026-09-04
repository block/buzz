import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const utilitiesCss = readFileSync(
  new URL("./utilities.css", import.meta.url),
  "utf8",
);
const huddleTooltipRule = utilitiesCss.match(
  /\.buzz-huddle-tooltip\s*\{([\s\S]*?)\n\s*\}/,
)?.[1];

test("huddle tooltips consume their dedicated semantic tokens", () => {
  assert.ok(huddleTooltipRule, "missing .buzz-huddle-tooltip rule");
  assert.match(
    huddleTooltipRule,
    /background:\s*hsl\(\s*var\(--huddle-tooltip-surface,/,
  );
  assert.match(
    huddleTooltipRule,
    /color:\s*hsl\(\s*var\(\s*--huddle-tooltip-foreground,/,
  );
  assert.doesNotMatch(
    huddleTooltipRule,
    /--(?:primary|secondary)(?:-foreground)?/,
  );
});
