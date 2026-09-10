#!/usr/bin/env node
/**
 * Fails the build when a text role cannot be read on a surface it is used on.
 *
 * This exists because the token table found four broken pairings by rendering
 * them, and a defect found by looking is a defect that ships until someone
 * looks. The table stays as the readable view; this is the check that runs.
 *
 * It parses `tokens.css` rather than a duplicate list of values, so it cannot
 * drift from the system it audits: resolve each role through its `var()` chain
 * to a literal, per mode, then measure every pairing the roles allow.
 *
 * Judged with APCA, per DESIGN.md § Contrast. The WCAG 2 ratio is reported for
 * context and never decides — it is the standard that calls #8f8f8f on #1c1c1c
 * a pass at 5.27:1 while APCA scores it Lc 40.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { apcaContrast, wcagRatio } from "./apca.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const TOKENS = join(ROOT, "src/shared/styles/tokens.css");

/** Body text. Anything a person must read to use the product. */
const TARGET_BODY = 60;
/** Large or non-essential text: timestamps, counts, meta. */
const TARGET_META = 45;

/**
 * Pairings that are deliberately below target, with the reason.
 *
 * An exception must name a role and say why, so the list stays short and
 * arguable rather than becoming a place to hide failures.
 *
 * A key is either a role — exempt on every surface — or `role on surface`,
 * exempt for that one pairing. Prefer the narrow form: a role-wide exemption
 * for a shortfall on one surface also silences a genuine failure on the next
 * surface that role reaches, which is the failure mode this list must not have.
 */
const EXCEPTIONS = new Map([
  [
    "--text-disabled",
    "Low contrast is the signal that a control is unavailable; WCAG exempts inactive controls. Never carries information a person needs.",
  ],
]);

/** Roles measured at the meta target rather than the body target. */
const META_ROLES = new Set(["--text-tertiary"]);

/**
 * Neutral surfaces any text may sit on.
 *
 * `--bg-hover` was a role and is now written as `--neutral-4`, so it is named by
 * its step here. It still belongs in this list: a row under the cursor is a
 * surface text sits on, whatever it is called.
 */
const SURFACES = ["--bg-panel", "--bg-float", "--neutral-2", "--neutral-4"];

/**
 * Text that must be readable on every neutral surface.
 *
 * **The palette steps in this list are the point, not an oversight.** Once a
 * screen may write `text-red-12` directly, the guard has to measure the step a
 * screen actually uses — otherwise it audits names nobody types. `--text-error`
 * and `--text-accent` used to be here; deleting the roles without moving their
 * steps into this list would have left the guard passing while measuring
 * nothing, which is precisely the failure that shipped error text at Lc 34.
 *
 * A step joins this list when a screen starts using it for text. `red-12` is
 * here because three error messages use it; `red-11` is not, because nothing
 * does — and it is the step that fails, at 59.7 on a dark panel.
 */
const TEXT_ROLES = [
  "--text-primary",
  "--text-secondary",
  "--text-tertiary",
  "--text-disabled",
  "--purple-12", // accent text: links, active nav, chip labels
  "--red-12", // error text: failed session start, rejected form
];

/**
 * Paired text and the fills it is defined against — including each fill's hover.
 *
 * The hover was the gap. A solid button had no `bg-*-hover`, so hovering one
 * fell through to the neutral `bg-hover` and painted white paired text on light
 * grey — Lc 13, invisible, and this guard could not see it because it only
 * measured the resting fill. A paired text colour is only correct if it clears
 * every fill it can actually sit on, and hover is one of them.
 */
const PAIRS = [
  ["--text-on-accent", "--purple-9"],
  ["--text-on-accent", "--purple-10"],
  // `bg-neutral-11` with `text-neutral-1` — the inverse pair, written as steps
  // now that the roles are gone. Still measured as a pair, because the text
  // follows the fill: move the fill and this has to be re-measured.
  ["--neutral-1", "--neutral-11"],
];

/**
 * Tint surfaces and the coloured text that sits on them, as steps.
 *
 * DESIGN.md: "tint carries a meaning and takes coloured text". The accent tint
 * and its hover are both real surfaces — a chip at rest and a chip under the
 * cursor — and the hover is the harder one, which is where the gap was.
 */
const TINT_PAIRS = [
  ["--purple-12", "--purple-3"],
  ["--purple-12", "--purple-4"],
];

const css = readFileSync(TOKENS, "utf8");

/**
 * Collects declarations per mode, honouring source order so a later block wins
 * exactly as the cascade would. `.dark` inherits `:root` and overrides it, so
 * dark is light-then-dark rather than dark alone.
 */
function declarationsByMode() {
  const light = new Map();
  const dark = new Map();
  // Split on top-level selector blocks and track which we are inside.
  const blocks = css.matchAll(/(:root|\.dark)\s*\{([^}]*)\}/g);
  for (const [, selector, body] of blocks) {
    for (const [, name, value] of body.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
      const v = value.trim();
      if (selector === ":root") {
        light.set(name, v);
        // Dark starts from the light value and is overridden below.
        dark.set(name, v);
      } else {
        dark.set(name, v);
      }
    }
  }
  return { light, dark };
}

/** Follows a `var()` chain to its literal, or null if it is not a plain colour. */
function resolve(map, name, depth = 0) {
  if (depth > 10) return null;
  const value = map.get(name);
  if (!value) return null;
  const single = /^var\(\s*(--[\w-]+)\s*\)$/.exec(value);
  if (single) return resolve(map, single[1], depth + 1);
  return /^#[0-9a-fA-F]{3,6}$/.test(value) ? value : null;
}

const modes = declarationsByMode();
const failures = [];
const skipped = [];
/**
 * Pairing-scoped exceptions this run actually needed. An exception nobody hits
 * is either fixed or wrong, and either way should not sit in the list
 * unchallenged — same reasoning as the type guard's stale-override report.
 */
const claimedExceptions = new Set();

for (const [mode, map] of Object.entries(modes)) {
  const check = (textRole, surfaceRole) => {
    const text = resolve(map, textRole);
    const surface = resolve(map, surfaceRole);
    // A translucent or gradient value cannot be measured against one colour;
    // record it rather than passing it silently.
    if (!text || !surface) {
      skipped.push(`${mode}: ${textRole} on ${surfaceRole}`);
      return;
    }
    const lc = Math.abs(apcaContrast(text, surface));
    const target = META_ROLES.has(textRole) ? TARGET_META : TARGET_BODY;
    if (lc >= target) return;
    // Role-wide first, then the narrow `role on surface` form.
    if (EXCEPTIONS.has(textRole)) return;
    if (EXCEPTIONS.has(`${textRole} on ${surfaceRole}`)) {
      claimedExceptions.add(`${textRole} on ${surfaceRole}`);
      return;
    }
    failures.push({
      mode,
      textRole,
      text,
      surfaceRole,
      surface,
      lc,
      target,
      wcag: wcagRatio(text, surface),
    });
  };

  for (const role of TEXT_ROLES) {
    for (const surface of SURFACES) check(role, surface);
  }
  for (const [role, fill] of PAIRS) check(role, fill);
  for (const [text, tint] of TINT_PAIRS) check(text, tint);
}

// Guard the maths itself: if these drift, every verdict above is wrong.
const REFERENCE = [
  ["#000000", "#ffffff", 106.04],
  ["#ffffff", "#000000", -107.88],
  ["#888888", "#ffffff", 63.06],
];
for (const [text, bg, expected] of REFERENCE) {
  const got = apcaContrast(text, bg);
  if (Math.abs(got - expected) > 0.05) {
    console.error(
      `✗ APCA implementation drifted: ${text} on ${bg} gave ${got.toFixed(2)}, expected ${expected}`,
    );
    process.exit(1);
  }
}

if (skipped.length > 0) {
  console.log(`ℹ ${skipped.length} pairing(s) not measurable as flat colour:`);
  for (const s of skipped) console.log(`    ${s}`);
}

if (failures.length > 0) {
  console.error(
    `\n✗ Contrast: ${failures.length} pairing(s) below their APCA target\n`,
  );
  for (const f of failures) {
    console.error(
      `  ${f.mode.padEnd(5)} ${f.textRole} (${f.text}) on ${f.surfaceRole} (${f.surface})`,
    );
    console.error(
      `        Lc ${f.lc.toFixed(1)} — needs ${f.target}   [WCAG ${f.wcag.toFixed(2)}:1]`,
    );
  }
  console.error(
    "\nRaise the ramp step the role points at, or add a documented exception\nin scripts/check-contrast.mjs with a reason. See DESIGN.md § Contrast.\n",
  );
  process.exit(1);
}

console.log("✓ Contrast: every text role clears its APCA target in both modes");
for (const [role, why] of EXCEPTIONS) {
  console.log(`  (exception) ${role} — ${why.split(";")[0]}`);
}

const stale = [...EXCEPTIONS.keys()].filter(
  (key) => key.includes(" on ") && !claimedExceptions.has(key),
);
if (stale.length > 0) {
  console.log("\nℹ Pairing exceptions no longer needed — delete them:");
  for (const key of stale) console.log(`  ${key}`);
}
