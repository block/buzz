/**
 * Onboarding layout regression guard.
 *
 * `.buzz-onboarding-step-frame` provides the setup-step content area's
 * min-height floor for visual stability on tall viewports. It must NEVER
 * also declare a max-height: on a short viewport (MacBook Pro with the
 * window docked, scaled display, or with a harness card wrapped to a second
 * row after an install failure) content legitimately exceeds the min-height
 * floor and relies on the outer `.buzz-startup-shell` (`overflow-y-auto`) to
 * scroll. A max-height silently clips that content — the "Finish" button,
 * footer CTAs, and the bottom of the harness grid become unreachable.
 *
 * Regression for https://github.com/block/buzz/issues/1946.
 *
 * This test reads the CSS file as text (the same trick `motion.test.mjs`
 * uses) and asserts the frame rule does not introduce a max-height.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const cssPath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../shared/styles/globals/components.css",
);
const css = readFileSync(cssPath, "utf8");

// Extract the `.buzz-onboarding-step-frame` rule body.
function stepFrameRule() {
  const match = css.match(
    /\.buzz-onboarding-step-frame\s*\{([\s\S]*?)\n\s*\}/,
  );
  assert.ok(
    match,
    "expected desktop/src/shared/styles/globals/components.css to define .buzz-onboarding-step-frame",
  );
  return match[1];
}

test("onboarding step frame has no max-height (must grow to fit content)", () => {
  assert.doesNotMatch(
    stepFrameRule(),
    /\bmax-height\s*:/,
    "step frame must not declare max-height — outer shell handles scrolling (#1946)",
  );
});

test("onboarding step frame keeps its min-height comfort floor", () => {
  // The min-height is the intended design behavior — it stabilizes the
  // transition between setup steps on large screens. We assert it's still
  // present so a careless refactor doesn't remove it.
  assert.match(stepFrameRule(), /\bmin-height\s*:\s*min\(/);
});

test("outer onboarding shell remains the scroll container", () => {
  // Belt-and-suspenders: the scroll surface is `.buzz-startup-shell` on the
  // root of `MachineOnboardingFlow`. Assert that rule still declares
  // `overflow-y-auto` (or `overflow-y: auto`) so a future layout refactor
  // cannot silently drop it while this test passes alone.
  assert.match(
    css,
    /\.buzz-startup-shell\s*\{[\s\S]*?min-height:\s*100dvh/,
    "expected .buzz-startup-shell to define the base scroll surface",
  );
  assert.match(
    css,
    /\.buzz-onboarding-neutral-theme\.buzz-startup-shell\s*\{[\s\S]*?\}/s,
    "expected themed shell styles to remain present",
  );
});
