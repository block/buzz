#!/usr/bin/env node
/**
 * Type-system guard.
 *
 * Four rules from DESIGN.md § Type, enforced rather than trusted. Each one has
 * already cost the existing client real work:
 *
 *   1. No arbitrary text sizes — `text-[15px]`, `text-[0.9rem]`, `font-size:`.
 *      Fixed px froze against keyboard zoom and shipped a message-timeline
 *      regression; arbitrary rem re-fragments the scale we just consolidated.
 *   2. No `uppercase` / `tracking-*` utilities. All-caps labels are less legible
 *      and read as enterprise chrome, and tracking is corrected per ramp step.
 *   3. No size role paired with a leading utility. A role carries its own line
 *      height; overriding it is how two identical labels drift apart.
 *   4. No weight outside 400 and 600. `font-semibold` is bold.
 *
 * What this guard is for: an agent building a screen has no reason to prefer
 * `font-medium` over `font-semibold`, so left unguided it picks either, and the
 * product ends up with four weights that mean nothing. The guard removes the
 * choice where there is no judgement to apply.
 *
 * What it is NOT for: stopping a designer. Every rule has a named override with
 * a reason, and adding one is a normal edit — not an escalation. A guard that
 * blocks real work gets deleted; a guard that asks you to say why does not.
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
  "body-lg",
  "body-sm",
  "body",
  "mono-lg",
  "mono-sm",
  "mono",
];

/**
 * Roles that existed before the ramp was sized from the product, with what to
 * use instead. Kept as a rule rather than deleted quietly: the names read
 * plausible, so a stale one would otherwise resolve to nothing and render at the
 * inherited size — a silent failure that looks like a design choice.
 */
const RETIRED_ROLES = new Map([
  ["subheading", "text-heading, or text-body-lg if it is prose"],
  ["label", "text-body, or text-body-sm in dense chrome"],
  ["caption", "text-body-sm"],
  ["meta", "text-body-sm"],
  ["code", "text-mono"],
]);

/**
 * Deliberate exceptions, as `relativePath:matchedText`.
 *
 * The escape hatch, and it is meant to be used. A one-off where another weight
 * genuinely looks better is a real thing; the only requirement is that it says
 * so out loud, so the next person reads a decision instead of guessing at an
 * accident. Matching the text rather than a line number keeps entries stable
 * when unrelated edits move code around.
 *
 * If this list starts growing in one direction — several `font-medium` entries,
 * say — that is the signal the system is missing a role, not that the rule is
 * wrong. Fix the system at that point rather than adding a tenth override.
 */
const OVERRIDES = new Map([
  // Example of the shape. Remove when a real exception replaces it.
  // ["features/foo/ui/Bar.tsx:font-medium", "Optical match to the adjacent
  //  native control, which renders at 500 and cannot be changed."],
]);

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
    id: "role-plus-leading",
    // A size role sharing a class attribute with a line-height utility. A role
    // carries its own leading, so overriding it here is how two supposedly
    // identical labels drift apart.
    //
    // Weight is deliberately NOT in this rule: `text-body font-semibold` is the
    // documented way to bold body text, because size is the paragraph's
    // decision and weight is the phrase's. Weight is policed by name below.
    pattern: new RegExp(
      `class(?:Name)?=(?:"|'|\`)[^"'\`]*\\btext-(?:${SIZE_ROLES.join("|")})\\b[^"'\`]*\\bleading-`,
      "g",
    ),
    message:
      "size role paired with a leading utility — a role already carries its own line height.",
  },
  {
    id: "off-ramp-weight",
    // The system has two weights: 400 (content) and 600 (structure and
    // emphasis). 500 was measured against 400 at body size and does not read as
    // intent in a scanned list — it is heavy enough to muddy a column and too
    // subtle to signal. 700 was rejected as louder than anything Buzz needs.
    // The rest have never had a use.
    //
    // `font-semibold` and `font-normal` are absent from this list on purpose:
    // they are the two legal weights.
    pattern: /\bfont-(?:thin|extralight|light|medium|bold|extrabold|black)\b/g,
    message:
      "off-ramp font weight — the system is 400 and 600. Bold is font-semibold. If a one-off genuinely needs another weight, add it to OVERRIDES with a reason.",
  },
  {
    id: "retired-role",
    // A retired size role. Tailwind emits no utility for these, so the element
    // silently renders at whatever it inherits — right often enough to pass a
    // glance, wrong wherever the inherited size differs.
    pattern: new RegExp(
      `\\btext-(?:${[...RETIRED_ROLES.keys()].join("|")})\\b`,
      "g",
    ),
    message: "retired size role",
    detail: (matched) => {
      const role = matched.replace("text-", "");
      return `use ${RETIRED_ROLES.get(role)} — this role no longer exists, so it resolves to nothing and inherits a size instead.`;
    },
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
/** Overrides actually hit this run, so stale entries can be reported. */
const used = new Set();

for (const file of walk(SRC)) {
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
      if (!match) continue;

      const rel = relative(SRC, file);
      if (OVERRIDES.has(`${rel}:${match[0]}`)) {
        used.add(`${rel}:${match[0]}`);
        continue;
      }

      const detail = rule.detail ? rule.detail(match[0]) : rule.message;
      failures.push(
        `${relative(process.cwd(), file)}:${index + 1}  ${rule.id}\n    ${trimmed}\n    → ${detail}`,
      );
    }
  });
}

// A stale override is worse than none: it silently permits whatever moves into
// its place. Report them rather than letting the list rot.
const stale = [...OVERRIDES.keys()].filter((k) => !used.has(k));
if (stale.length > 0) {
  console.warn(
    `⚠ ${stale.length} unused override${stale.length === 1 ? "" : "s"} in check-type.mjs — remove ${stale.length === 1 ? "it" : "them"}:`,
  );
  for (const k of stale) console.warn(`    ${k}`);
}

if (failures.length > 0) {
  console.error(
    `\n✗ Type system: ${failures.length} violation${failures.length === 1 ? "" : "s"}\n`,
  );
  console.error(`${failures.join("\n\n")}\n`);
  process.exit(1);
}

console.log("✓ Type system: no violations");
