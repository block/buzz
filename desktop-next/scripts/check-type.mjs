#!/usr/bin/env node
/**
 * Type-system guard.
 *
 * Three rules from DESIGN.md § Type, enforced rather than trusted. Each one has
 * already cost the existing client real work:
 *
 *   1. No arbitrary text sizes — `text-[15px]`, `text-[0.9rem]`, `font-size:`.
 *      Fixed px froze against keyboard zoom and shipped a message-timeline
 *      regression; arbitrary rem re-fragments the scale we just consolidated.
 *   2. No `uppercase` / `tracking-*` utilities. All-caps labels are less legible
 *      and read as enterprise chrome, and tracking is corrected per ramp step.
 *   3. No size role paired with a weight or leading utility. A role carries its
 *      whole setting; pairing one with `font-medium` is how two supposedly
 *      identical labels drift apart.
 *
 * Run: pnpm check:type
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const SRC = new URL("../src", import.meta.url).pathname;

/** Size roles a component may use. Kept in sync with typography.css. */
const SIZE_ROLES = [
  "display",
  "title",
  "heading",
  "subheading",
  "body-lg",
  "body",
  "label",
  "caption",
  "meta",
  "code",
];

const RULES = [
  {
    id: "arbitrary-text-size",
    // text-[...] with any unit, plus raw CSS font-size declarations.
    pattern: /\btext-\[[^\]]*(?:px|rem|em|pt|%)[^\]]*\]|font-size\s*:/g,
    message:
      "arbitrary text size — use a named role (text-body, text-label, …). px freezes against zoom; arbitrary rem re-fragments the scale.",
  },
  {
    id: "uppercase",
    // Only inside a className/class string — otherwise prose describing the
    // rule trips it, and a guard that cries wolf gets ignored.
    pattern: /class(?:Name)?=(?:"|'|`)[^"'`]*\buppercase\b/g,
    message:
      "all-caps text — DESIGN.md § Type forbids it. A quiet label uses text-meta on text-tertiary instead.",
  },
  {
    id: "manual-tracking",
    pattern: /\btracking-(?:tighter|tight|normal|wide|wider|widest|\[)/g,
    message:
      "hand-applied tracking — the ramp already corrects tracking per step.",
  },
  {
    id: "role-plus-weight",
    // A size role sharing a *className string* with a weight or leading
    // utility. Anchoring to the attribute keeps the ramp's own
    // `--text-body--line-height` declarations out of it — those are the
    // definitions, not a component overriding one.
    pattern: new RegExp(
      `class(?:Name)?=(?:"|'|\`)[^"'\`]*\\btext-(?:${SIZE_ROLES.join("|")})\\b[^"'\`]*\\b(?:font-(?:thin|extralight|light|normal|medium|semibold|bold|extrabold|black)|leading-)`,
      "g",
    ),
    message:
      "size role paired with a weight/leading utility — a role already carries its whole setting.",
  },
];

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) return walk(full);
    return /\.(tsx?|css)$/.test(full) ? [full] : [];
  });
}

const failures = [];

for (const file of [
  ...walk(SRC),
  ...walk(new URL("../../packages/ui/src", import.meta.url).pathname),
]) {
  const source = readFileSync(file, "utf8");
  // The typography ramp itself is the one place literals are legal — it is
  // layer 1, where values live by design.
  const isRamp = file.endsWith("typography.css");
  const lines = source.split("\n");

  lines.forEach((line, index) => {
    // Skip comment-only lines: the docs quote the very patterns they forbid.
    const trimmed = line.trim();
    if (
      trimmed.startsWith("*") ||
      trimmed.startsWith("//") ||
      trimmed.startsWith("/*")
    ) {
      return;
    }

    for (const rule of RULES) {
      if (isRamp && rule.id === "arbitrary-text-size") continue;
      rule.pattern.lastIndex = 0;
      const match = rule.pattern.exec(line);
      if (match) {
        failures.push(
          `${relative(process.cwd(), file)}:${index + 1}  ${rule.id}\n    ${trimmed}\n    → ${rule.message}`,
        );
      }
    }
  });
}

if (failures.length > 0) {
  console.error(
    `\n✗ Type system: ${failures.length} violation${failures.length === 1 ? "" : "s"}\n`,
  );
  console.error(`${failures.join("\n\n")}\n`);
  process.exit(1);
}

console.log("✓ Type system: no violations");
