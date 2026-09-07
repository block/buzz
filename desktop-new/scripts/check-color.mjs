#!/usr/bin/env node
/**
 * Colour-system guard.
 *
 * One rule, and it exists because of a specific near-miss: the accent tint
 * looked right in light mode at Tailwind's `purple-100`, but its dark
 * counterpart looked oversaturated, and the obvious fix was to write
 * `purple-950/50` — dim a too-strong colour until it looks subtle.
 *
 * That value was not wrong. `purple-950/50` composites to roughly the same
 * muted deep purple the palette now holds as a step, and two independent
 * attempts landed within a few percent of it. The problem is where it lives: an
 * opacity expression inside a component is a colour decision with no name, no
 * light/dark pair, and no way for the contrast guard to measure it.
 *
 * So the rule is not "never use opacity". It is: **opacity is not how you reach
 * a lighter or subtler colour.** If a step is missing, add it to the palette —
 * that is a reviewed diff instead of a value buried in a class list.
 *
 * Genuine translucency, where something behind must show through, is a
 * different axis and has its own tokens: `glass-*`, and the `--texture-*` and
 * shadow values that bake alpha into a literal.
 *
 * It also audits the layering itself: that the palette is the only place a
 * literal lives, that a family step references a palette step rather than
 * holding its own value, and that two tokens doing the same job resolve to the
 * same step rather than merely to the same value today. That last one is not
 * hypothetical — `accent-2` and `tint-purple` were the same purple in light mode
 * and two different purples in dark, and nothing caught it because both were
 * hand-picked.
 *
 * Run: pnpm check:color
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const SRC = new URL("../src", import.meta.url).pathname;
const TOKENS_FILE = new URL("../src/shared/styles/tokens.css", import.meta.url)
  .pathname;

/**
 * Colour utilities that may not carry an opacity modifier.
 *
 * Deliberately the colour-bearing prefixes only. `opacity-50` on a whole
 * element is a legitimate way to fade a thing out, and `bg-black/50` on a
 * scrim is genuine translucency — the first is not a colour choice and the
 * second is caught by review, not by this guard.
 */
const COLOR_PREFIXES = [
  "bg",
  "text",
  "border",
  "ring",
  "fill",
  "stroke",
  "shadow",
  "outline",
  "divide",
  "decoration",
  "accent",
  "caret",
  "from",
  "via",
  "to",
];

/**
 * `bg-accent-tint/50`, `text-primary/80`. The slash-number form is Tailwind's
 * opacity modifier, and on a colour utility it always means "I want a different
 * shade of this".
 */
const OPACITY_MODIFIER = new RegExp(
  String.raw`\b(?:${COLOR_PREFIXES.join("|")})-[a-z0-9-]+\/(\d{1,3}|\[[^\]]+\])`,
  "g",
);

/**
 * `color-mix(in srgb, var(--x) 50%, transparent)` and friends: the same move
 * spelled in CSS. Also catches raw `rgba()`/`hsla()` with a fractional alpha on
 * a colour property.
 */
const CSS_MIXERS = [
  { pattern: /color-mix\s*\(/g, what: "color-mix()" },
  { pattern: /\brgba?\([^)]*\/\s*0?\.\d+\s*\)/g, what: "rgb() with alpha" },
  { pattern: /\brgba\([^)]*,\s*0?\.\d+\s*\)/g, what: "rgba()" },
  { pattern: /\bhsla?\([^)]*[,/]\s*0?\.\d+\s*\)/g, what: "hsl() with alpha" },
];

/**
 * A component may not name a palette or family token directly. Only the role
 * layer is public — the palette is where values live and families are the jobs
 * a hue does, but a screen is built from what a thing *is*.
 */
const PRIVATE_TOKEN =
  /var\(\s*--(palette-[a-z]+-\d+|(?:accent|danger|success|warning|info)-(?:tint|tint-hover|border|fill|text)|neutral-\d+|glass-\d+)\s*\)/g;

/**
 * Files exempt from the private-token rule, with the reason.
 *
 * The token file defines the layers, so it necessarily references them. The
 * design-system pages exist to *show* the ramps, so they resolve token names on
 * purpose — that is their subject matter, not a shortcut.
 */
const PRIVATE_TOKEN_ALLOWED = new Map([
  ["shared/styles/tokens.css", "Defines the layers it references."],
  [
    "shared/tokens/registry.ts",
    "Documents the ramps; the token names are its content.",
  ],
  [
    "features/design-system/useResolvedToken.ts",
    "Resolves ramp steps so the design-system pages can display them.",
  ],
]);

/**
 * Files where building a colour value IS the job, with the reason.
 *
 * The token file is the one place alpha is legitimately baked into a literal —
 * glass fills, dot textures, shadow colours. That is the documented other side
 * of the rule: transparency for genuine see-through lives inside a value, at
 * the bottom layer, named.
 */
const MIXER_ALLOWED = new Map([
  [
    "shared/styles/tokens.css",
    "Bakes alpha into named values (glass, texture, shadow) — the layer where that belongs.",
  ],
]);

/** Specific `path:line` escapes, each with a reason. */
const OVERRIDES = new Map();

const failures = [];

function walk(dir) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      walk(full);
      continue;
    }
    if (!/\.(tsx?|css)$/.test(entry)) continue;
    check(full);
  }
}

function check(file) {
  const rel = relative(SRC, file);
  const lines = readFileSync(file, "utf8").split("\n");

  lines.forEach((line, index) => {
    const at = `${rel}:${index + 1}`;
    if (OVERRIDES.has(at)) return;
    // A comment explaining the rule is not a violation of it.
    const code = line.replace(/\/\*.*?\*\//g, "").replace(/\/\/.*$/, "");

    for (const match of code.matchAll(OPACITY_MODIFIER)) {
      failures.push({
        at,
        found: match[0],
        why: "Opacity is not how you reach a subtler colour. Add a palette step, or use a different step.",
      });
    }

    for (const { pattern, what } of MIXER_ALLOWED.has(rel) ? [] : CSS_MIXERS) {
      for (const match of code.matchAll(pattern)) {
        failures.push({
          at,
          found: match[0],
          why: `${what} builds a colour instead of naming one. Add a palette step; bake alpha into a value only for genuine translucency.`,
        });
      }
    }

    if (!PRIVATE_TOKEN_ALLOWED.has(rel)) {
      for (const match of code.matchAll(PRIVATE_TOKEN)) {
        failures.push({
          at,
          found: match[0],
          why: "Palette and family tokens are private. Build screens from role tokens.",
        });
      }
    }
  });
}

walk(SRC);
auditLayers();

/**
 * The layering rules, measured against `tokens.css` rather than a copied list,
 * so this cannot drift from the system it audits.
 */
function auditLayers() {
  const css = readFileSync(TOKENS_FILE, "utf8");
  const rel = "shared/styles/tokens.css";

  /** Every `:root` or `.dark` block, concatenated. One per layer, so several. */
  const blocksFor = (selector) => {
    const out = [];
    const pattern = new RegExp(`${selector.replace(".", "\\.")}\\s*\\{`, "g");
    for (const match of css.matchAll(pattern)) {
      let depth = 1;
      let i = match.index + match[0].length;
      const start = i;
      while (i < css.length && depth > 0) {
        if (css[i] === "{") depth += 1;
        else if (css[i] === "}") depth -= 1;
        i += 1;
      }
      out.push(css.slice(start, i - 1));
    }
    return out.join("\n");
  };

  const modes = { light: blocksFor(":root"), dark: blocksFor(".dark") };
  const read = (block, name) =>
    new RegExp(`^\\s*${name}:\\s*([^;]+);`, "m").exec(block)?.[1].trim();

  const HUES = [
    "gray",
    "purple",
    "red",
    "green",
    "amber",
    "blue",
    "cyan",
    "orange",
  ];
  const FAMILIES = ["accent", "danger", "success", "warning", "info"];
  const JOBS = ["tint", "tint-hover", "border", "fill", "text"];

  // 1. Every palette step exists in both modes and holds a literal.
  for (const hue of HUES) {
    for (let step = 1; step <= 12; step += 1) {
      const name = `--palette-${hue}-${step}`;
      for (const [mode, block] of Object.entries(modes)) {
        const value = read(block, name);
        if (!value) {
          failures.push({
            at: `${rel} (${mode})`,
            found: name,
            why: "Missing. Every palette step must be authored in both modes.",
          });
        } else if (!/^#[0-9a-f]{6}$/i.test(value)) {
          failures.push({
            at: `${rel} (${mode})`,
            found: `${name}: ${value}`,
            why: "A palette step holds a literal. Values live here and nowhere else.",
          });
        }
      }
    }
  }

  // 2. A family step references the palette; it never holds a value.
  for (const family of FAMILIES) {
    for (const job of JOBS) {
      const name = `--${family}-${job}`;
      for (const [mode, block] of Object.entries(modes)) {
        const value = read(block, name);
        // Absent in dark is correct: it inherits through the palette.
        if (!value) continue;
        if (!/^var\(--palette-[a-z]+-\d+\)$/.test(value)) {
          failures.push({
            at: `${rel} (${mode})`,
            found: `${name}: ${value}`,
            why: "A family step must reference a palette step, not hold a value.",
          });
        }
      }
    }
  }

  // 3. No two family steps resolve to the same palette step.
  //
  // The generalised form of the bug that motivated this layer. Two tokens that
  // hold the same value are either one job with two names — which drifts the
  // moment someone edits one — or two jobs that will become indistinguishable
  // on screen. Either way it wants a person to look. This catches it for any
  // future family without naming pairs, which the previous version did and
  // which went silently vacuous when those tokens were deleted.
  const seen = new Map();
  for (const family of FAMILIES) {
    for (const job of JOBS) {
      const name = `--${family}-${job}`;
      const value = read(modes.light, name);
      if (!value) continue;
      const existing = seen.get(value);
      if (existing) {
        failures.push({
          at: rel,
          found: `${existing} and ${name} both resolve to ${value}`,
          why: "Two family steps on one palette step. Either it is one job with two names, or two jobs that will look identical. Give the second its own step.",
        });
      } else {
        seen.set(value, name);
      }
    }
  }

  // 4. Dark restates the palette, never the families.
  for (const family of FAMILIES) {
    for (const job of JOBS) {
      if (read(modes.dark, `--${family}-${job}`)) {
        failures.push({
          at: `${rel} (dark)`,
          found: `--${family}-${job}`,
          why: "Restated in dark. A family inherits through the palette, which dark already redefines; restating it reopens the drift surface.",
        });
      }
    }
  }
}

if (failures.length === 0) {
  console.log(
    "✓ Colour: layers intact, no colour built by opacity, no private token used",
  );
  for (const [path, reason] of new Map([
    ...PRIVATE_TOKEN_ALLOWED,
    ...MIXER_ALLOWED,
  ])) {
    console.log(`  (exception) ${path} — ${reason}`);
  }
  process.exit(0);
}

console.error(`✗ Colour: ${failures.length} violation(s)\n`);
for (const { at, found, why } of failures) {
  console.error(`  ${at}`);
  console.error(`        ${found}`);
  console.error(`        ${why}\n`);
}
console.error(
  "Add the step to the palette in tokens.css, or add a documented override\nin scripts/check-color.mjs with a reason. See DESIGN.md § Colour.",
);
process.exit(1);
