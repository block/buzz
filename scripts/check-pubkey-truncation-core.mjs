import { promises as fs } from "node:fs";
import path from "node:path";

/**
 * Shared "no hand-rolled pubkey truncation" guard.
 *
 * A truncated pubkey prefix is forgeable by vanity-grinding, so display
 * truncation must be consistent and centralized: the canonical
 * `truncatePubkey` in `shared/lib/pubkey.ts` (or the `<PubKey>` component,
 * which also offers full-key reveal + copy). Ad-hoc `pubkey.slice(0, N)`
 * display forms fragmented into five formats before this guard existed.
 *
 * It flags `.slice(` / `.substring(` / `.slice(0` template-truncations applied
 * to identifiers that look like a pubkey/npub, outside the canonical module.
 * Non-display uses (array windows, color derivation from a key, avatar
 * initials) live in each app's `overrides` allowlist.
 */

const PUBKEY_SLICE_RE =
  /\b[A-Za-z_$][\w$]*(?:[Pp]ubkey|[Pp]ub_key|[Nn]pub)[\w$]*\??\.(?:slice|substring)\(|\b(?:pubkey|npub)\??\.(?:slice|substring)\(/g;

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
 * @param {Set<string>} [options.allowedFiles] Relative paths allowed to truncate (the canonical module).
 * @param {string} options.scriptPath Path mentioned in the failure hint.
 */
export async function runPubkeyTruncationCheck({
  projectRoot,
  rules,
  label,
  overrides = new Set(),
  allowedFiles = new Set(),
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
    if (!rule || !rule.extensions.has(path.extname(filePath))) {
      continue;
    }
    if (allowedFiles.has(relativePath.split(path.sep).join("/"))) {
      continue;
    }
    if (relativePath.includes(".test.")) {
      continue;
    }

    const content = await fs.readFile(filePath, "utf8");
    const lines = content.split("\n");
    lines.forEach((line, index) => {
      PUBKEY_SLICE_RE.lastIndex = 0;
      if (!PUBKEY_SLICE_RE.test(line)) {
        return;
      }
      const key = `${relativePath.split(path.sep).join("/")}:${index + 1}`;
      if (overrides.has(key)) {
        return;
      }
      violations.push({ key, line: line.trim() });
    });
  }

  if (violations.length > 0) {
    console.error(
      `${label}: found ${violations.length} hand-rolled pubkey truncation(s).\n` +
        `Use \`truncatePubkey\` from shared/lib/pubkey (or the <PubKey> component) instead.\n` +
        `Genuine non-display uses can be allowlisted in ${scriptPath}.\n`,
    );
    for (const violation of violations) {
      console.error(`  ${violation.key}: ${violation.line}`);
    }
    process.exit(1);
  }
}
