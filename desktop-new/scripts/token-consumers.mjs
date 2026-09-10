/**
 * Census: who actually reads each colour role?
 *
 * Not a guard — it never fails a build, and it is not in `pnpm check`. Run it by
 * hand when deciding whether a role earns its name:
 *
 *     node scripts/token-consumers.mjs
 *
 * It is deliberately advisory. "Zero consumers" is an argument for deleting a
 * role, not a verdict: a token can be legitimately unused for a week because the
 * screen that needs it is half-built. What it does do is make the argument
 * checkable, which is how the role layer went from 54 names to 29 — eighteen had
 * no reader anywhere, and six were read only by the pages documenting them.
 *
 * It counts two kinds of reader, because a role can be reached two ways:
 *
 *   1. `var(--bg-info)` in a stylesheet
 *   2. the Tailwind utility the role registers as — `bg-info` in a component
 *
 * Documentation is counted separately from product code, and that separation is
 * the point: a /design page rendering a swatch of `bg-info` proves the token
 * exists and nothing more. Nineteen of the twenty deleted status roles were
 * "used" in exactly that sense.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const SRC = join(ROOT, "src");
const TOKENS = join(SRC, "shared/styles/tokens.css");

/** Files that describe the system rather than use it. */
const isDocs = (path) =>
  path.includes("features/design-system/") ||
  path.includes("shared/styles/design-components.css") ||
  path.includes("shared/tokens/registry.ts");

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.(css|tsx?|ts)$/.test(full)) out.push(full);
  }
  return out;
}

const css = readFileSync(TOKENS, "utf8");

// Role declarations live outside `@theme`; registrations live inside it.
const themeStart = css.indexOf("@theme inline {");
const roleBlock = css.slice(0, themeStart);
const themeBlock = css.slice(themeStart);

const roles = new Set(
  [...roleBlock.matchAll(/^\s*(--(?:bg|text|border)-[a-z0-9-]+):/gm)].map(
    (m) => m[1],
  ),
);

// What Tailwind class does each role register as? `--color-info: var(--bg-info)`
// means `bg-info` / `text-info` reach `--bg-info`.
// Order matters: `--text-color-error` must match the `text-color` namespace, not
// `text` with the suffix `color-error`. Longest prefix first.
const NAMESPACES = ["text-color", "border-color", "color"];
const registrations = new Map(); // role -> [{ ns, suffix }]
for (const [, ns, suffix, role] of themeBlock.matchAll(
  new RegExp(
    String.raw`^\s*--(${NAMESPACES.join("|")})-([a-z0-9-]+):\s*var\((--[a-z0-9-]+)\)`,
    "gm",
  ),
)) {
  if (!registrations.has(role)) registrations.set(role, []);
  registrations.get(role).push({ ns, suffix });
}

const files = walk(SRC).filter((f) => f !== TOKENS);
const counts = new Map([...roles].map((r) => [r, { product: [], docs: [] }]));

for (const file of files) {
  const text = readFileSync(file, "utf8");
  const rel = relative(SRC, file);
  const bucket = isDocs(file) ? "docs" : "product";

  for (const role of roles) {
    const hits = [];
    if (text.includes(`var(${role})`)) hits.push("var()");

    for (const { ns, suffix } of registrations.get(role) ?? []) {
      // `--color-info` is reachable as bg-info, text-info, border-info…
      const prefixes =
        ns === "color"
          ? [
              "bg",
              "text",
              "border",
              "fill",
              "stroke",
              "ring",
              "outline",
              "shadow",
              "from",
              "to",
              "via",
            ]
          : ns === "border-color"
            ? ["border"]
            : ["text"]; // text-color
      for (const p of prefixes) {
        // Word-bounded so `bg-info` does not match `bg-info-tint`.
        if (new RegExp(`\\b(?:[a-z-]+:)?${p}-${suffix}\\b(?!-)`).test(text)) {
          hits.push(`${p}-${suffix}`);
          break;
        }
      }
    }

    if (hits.length)
      counts
        .get(role)
        [bucket].push(`${rel} (${[...new Set(hits)].join(", ")})`);
  }
}

const dead = [];
const docsOnly = [];
const live = [];
for (const [role, { product, docs }] of counts) {
  if (product.length) live.push([role, product]);
  else if (docs.length) docsOnly.push([role, docs]);
  else dead.push(role);
}

console.log(`\n${roles.size} roles declared.\n`);
console.log(`── Zero readers anywhere (${dead.length}) ──`);
for (const role of dead.sort()) console.log(`  ${role}`);
console.log(`\n── Read only by the docs (${docsOnly.length}) ──`);
for (const [role, where] of docsOnly.sort())
  console.log(`  ${role}  ← ${where.join("; ")}`);
console.log(`\n── Read by product code (${live.length}) ──`);
for (const [role, where] of live.sort((a, b) => a[1].length - b[1].length)) {
  console.log(`  ${role}  ×${where.length}`);
  for (const w of where) console.log(`      ${w}`);
}
