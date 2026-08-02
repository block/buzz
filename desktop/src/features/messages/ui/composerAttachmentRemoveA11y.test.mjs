/**
 * Regression guard for #2389 — composer attachment remove buttons must be
 * reachable by keyboard, not only via mouse hover.
 *
 * History: the remove (×) button on every composer attachment chip used
 * `hidden ... group-hover:flex`, which is `display: none` until the parent
 * chip is hovered. Because the button never appears in the tab order, keyboard
 * users could not reach it at all — there was no way to remove an attached
 * file without a mouse.
 *
 * Fix: add `group-focus-within:flex` alongside `group-hover:flex` so the
 * button renders (and is therefore tabbable) whenever ANY focusable element
 * inside the chip — including the button itself — has focus. Also adds an
 * explicit `aria-label="Remove attachment"` so screen readers announce the
 * action before relying on the tooltip.
 *
 * This test reads ComposerAttachments.tsx and asserts the focus-within half
 * of the fix is present on every chip-level remove button. If a refactor
 * ever drops the `group-focus-within:flex` class or the aria-label, this
 * test fails before the a11y regression ships.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(
  join(here, "ComposerAttachments.tsx"),
  "utf8",
);

test("composer remove-attachment buttons are focus-within reachable", () => {
  // Match every chip-level remove button block; the group-focus-within:flex
  // class is what puts the button back in the tab order.
  const focusablePattern =
    /aria-label="Remove attachment"[\s\S]*?group-hover:flex group-focus-within:flex/g;
  const matches = source.match(focusablePattern) ?? [];
  assert.ok(
    matches.length >= 2,
    `expected at least 2 remove-attachment buttons with focus-within fix ` +
      `(image/video + file chip), found ${matches.length}`,
  );
});

test("every `hidden ... group-hover:flex` remove-button also has focus-within", () => {
  // Defensive: if a new chip variant adds another `hidden ... group-hover:flex`
  // button without pairing it with `group-focus-within:flex`, fail loudly.
  const hoverOnlyPattern =
    /className="[^"]*\bhidden\b[^"]*\bgroup-hover:flex\b(?!\s+group-focus-within:flex)[^"]*"/g;
  const hoverOnlyMatch = source.match(hoverOnlyPattern);
  assert.equal(
    hoverOnlyMatch,
    null,
    "found a hidden/group-hover:flex button that is missing group-focus-within:flex — " +
      "keyboard users would lose access to it",
  );
});

test("remove-attachment buttons expose an aria-label", () => {
  // Tooltip alone is not a sufficient accessible name for an icon-only button.
  const ariaCount = (source.match(/aria-label="Remove attachment"/g) ?? [])
    .length;
  assert.ok(
    ariaCount >= 2,
    `expected at least 2 aria-labeled remove buttons, found ${ariaCount}`,
  );
});
