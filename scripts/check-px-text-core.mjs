import { promises as fs } from "node:fs";
import path from "node:path";

/**
 * Shared "no hardcoded px text size" guard.
 *
 * Zoom (Cmd +/-) scales the root <html> font-size, so only **rem**-based text
 * scales. Hardcoded px text sizes (`text-[15px]`, `font-size: 15px`) freeze
 * against zoom — that's the timeline regression we fixed. This guard stops new
 * px text sizes from creeping back in. Use a rem-based Tailwind token instead
 * (e.g. the stock `text-base`, `text-sm`, `text-xs` scale — chat === base).
 *
 * It flags:
 *   - Tailwind arbitrary text-size utilities: `text-[NNpx]`, `text-[N.NNrem]`,
 *     `text-[N.NNem]` — any arbitrary font-size literal. Use a named token
 *     (`text-2xs`, `text-3xs`, or the stock `text-base`/`text-sm`/`text-xs`
 *     scale) so the size lives in `tailwind.config.js` as rem and stays
 *     consistent. px literals freeze against zoom; arbitrary rem literals
 *     re-fragment the scale we just consolidated.
 *   - CSS px font sizes: `font-size: NNpx`
 *
 * Decorative/chrome exceptions (avatar initials sized to a fixed avatar box,
 * the `text-[6rem]` emoji glyph, etc.) live in the `overrides` allowlist
 * supplied by each app.
 */

// Any arbitrary Tailwind text-size literal — px, rem, or em. Color literals
// like `text-[#fff]` or `text-[var(--x)]` don't match (no unit-bearing number).
const TEXT_ARBITRARY_RE = /\btext-\[\d+(?:\.\d+)?(?:px|rem|em)\]/g;
// Match the CSS `font-size` property, but NOT custom properties like
// `--font-size:` (third-party widget vars) which merely contain the substring.
const FONT_SIZE_PX_RE = /(?<!-)\bfont-size:\s*\d+(?:\.\d+)?px/g;

async function walkFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        return walkFiles(fullPath);
      }
      return [fullPath];
    }),
  );
  return files.flat();
}

/**
 * @param {object} options
 * @param {string} options.projectRoot Absolute path the rule roots resolve against.
 * @param {Array<{root: string, extensions: Set<string>}>} options.rules Where to scan.
 * @param {string} options.label Human label for the failure header.
 * @param {Set<string>} [options.overrides] Allowlisted "relativePath:lineNumber" entries.
 * @param {string} options.scriptPath Path mentioned in the failure hint.
 */
export async function runPxTextCheck({
  projectRoot,
  rules,
  label,
  overrides = new Set(),
  scriptPath,
}) {
  const candidateFiles = (
    await Promise.all(
      rules.map((rule) => {
        const dir = path.join(projectRoot, rule.root);
        return fs
          .access(dir)
          .then(() => walkFiles(dir))
          .catch(() => []);
      }),
    )
  ).flat();

  const violations = [];

  for (const filePath of candidateFiles) {
    const relativePath = path.relative(projectRoot, filePath);
    const rule = rules.find((r) =>
      relativePath.startsWith(`${r.root}${path.sep}`),
    );
    if (!rule) {
      continue;
    }
    if (!rule.extensions.has(path.extname(relativePath))) {
      continue;
    }
    // Optional per-rule basename allowlist — scopes the scan to specific files.
    if (rule.files && !rule.files.has(path.basename(relativePath))) {
      continue;
    }

    const content = await fs.readFile(filePath, "utf8");
    const lines = content.split(/\r?\n/);
    lines.forEach((line, index) => {
      const lineNumber = index + 1;
      const key = `${relativePath}:${lineNumber}`;
      if (overrides.has(key)) {
        return;
      }
      const matches = [
        ...(line.match(TEXT_ARBITRARY_RE) ?? []),
        ...(line.match(FONT_SIZE_PX_RE) ?? []),
      ];
      for (const match of matches) {
        violations.push({ relativePath, lineNumber, match });
      }
    });
  }

  if (violations.length > 0) {
    console.error(`${label} px-text check failed:`);
    for (const v of violations) {
      console.error(`- ${v.relativePath}:${v.lineNumber}: ${v.match}`);
    }
    console.error(
      "Use a rem-based Tailwind text token (e.g. the stock `text-base`, " +
        "`text-sm`, `text-xs` scale, or the `text-2xs` / `text-3xs` meta-text " +
        "tokens) so the text scales with Cmd +/- zoom and stays on one scale. " +
        "If this size is " +
        "genuinely decorative/chrome (not readable message text), add a " +
        `narrowly scoped \`relativePath:lineNumber\` exception in \`${scriptPath}\`.`,
    );
    process.exit(1);
  }
}
