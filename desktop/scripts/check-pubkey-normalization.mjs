import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Guards how an agent pubkey is normalized before it is compared or used as a
 * map key.
 *
 * `normalizePubkey` (src/shared/lib/pubkey.ts) is `trim().toLowerCase()`. A
 * bare `.toLowerCase()` agrees with it for every well-formed pubkey and
 * disagrees the moment one carries surrounding whitespace — so the two forms
 * silently diverge exactly where a value arrived from somewhere untidy (a
 * pasted allowlist entry, a relay tag, a config file). The failure mode is the
 * one this feature keeps producing: two surfaces answer "is this the same
 * agent?" differently, nothing throws, and something merely stops matching.
 *
 * `RespondToField.handleRemove` was a live instance before this guard existed:
 * it compared `p.toLowerCase()` against a normalized pubkey, so one side
 * trimmed and the other did not.
 *
 * Sibling of `check-agent-identity.mjs`. That one guards how an identity KEY is
 * minted; this one guards how the pubkey inside it is normalized. Both exist
 * because the drift is invisible until a user reports that an agent "isn't
 * there".
 *
 * ## Why this is scoped to `src/features/agents`
 *
 * Hand-rolled `.toLowerCase()` on a pubkey is repo-wide — 87 files across
 * `desktop/src` at the time of writing, against 113 that use `normalizePubkey`.
 * Failing on all of them would make this guard unshippable, and a guard that
 * has to be disabled to land anything protects nothing.
 *
 * So it covers the surface whose divergence actually caused an outage, and
 * covers it completely — no allowlist. Widening it is a follow-up that has to
 * come with the call-site fixes, not a flag flip.
 */

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

/** The module that defines normalization; it is where `.toLowerCase()` belongs. */
const CANONICAL_MODULE = "src/shared/lib/pubkey.ts";

const SCAN_ROOT = "src/features/agents";
const EXTENSIONS = new Set([".ts", ".tsx"]);

// Any dotted or optionally-chained expression with `.toLowerCase()` applied to
// it. Matching broadly and then filtering on the receiver (see `receiverOf`) is
// deliberate: an earlier version anchored the pubkey inside the pattern and
// could not see `agent?.pubkey.toLowerCase()`, because the optional link sat
// between the identifier and the segment being matched. Ordinary case-folding —
// display names, commands, file paths — is excluded by the receiver test, not
// by the regex.
const LOWERCASE_CALL_RE =
  /[A-Za-z_$][\w$]*(?:\??\.[A-Za-z_$][\w$]*)*\??\.toLowerCase\(\)/g;

/** Reads as a pubkey, so `normalizePubkey` is the correct normalizer for it. */
const PUBKEY_RECEIVER_RE = /pubkey/i;

/** The receiver an expression applies `.toLowerCase()` to. */
function receiverOf(match) {
  return match.replace(/\??\.toLowerCase\(\)$/, "");
}

/** A line that is wholly a comment documents a key format; it does not build one. */
function isCommentLine(line) {
  const trimmed = line.trimStart();
  return (
    trimmed.startsWith("//") ||
    trimmed.startsWith("/*") ||
    trimmed.startsWith("*")
  );
}

async function walkFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(directory, entry.name);
      return entry.isDirectory() ? walkFiles(fullPath) : [fullPath];
    }),
  );
  return files.flat();
}

const scanDirectory = path.join(projectRoot, SCAN_ROOT);
const candidateFiles = await fs
  .access(scanDirectory)
  .then(() => walkFiles(scanDirectory))
  .catch(() => []);

const violations = [];

for (const filePath of candidateFiles) {
  // Authored with `/`, but path.relative yields `\` on Windows — compare in
  // posix form or the canonical-module skip silently never matches.
  const relativePath = path
    .relative(projectRoot, filePath)
    .split(path.sep)
    .join("/");

  if (!EXTENSIONS.has(path.extname(relativePath))) continue;
  if (relativePath === CANONICAL_MODULE) continue;

  const content = await fs.readFile(filePath, "utf8");
  content.split(/\r?\n/).forEach((line, index) => {
    if (isCommentLine(line)) return;
    for (const match of line.match(LOWERCASE_CALL_RE) ?? []) {
      if (!PUBKEY_RECEIVER_RE.test(receiverOf(match))) continue;
      violations.push({ relativePath, lineNumber: index + 1, match });
    }
  });
}

if (violations.length > 0) {
  console.error("Desktop pubkey-normalization check failed:");
  for (const violation of violations) {
    console.error(
      `- ${violation.relativePath}:${violation.lineNumber}: ${violation.match}`,
    );
  }
  console.error(
    "Normalize agent pubkeys with `normalizePubkey` from " +
      `\`${CANONICAL_MODULE}\`, not a bare \`.toLowerCase()\`. The two agree ` +
      "until a pubkey arrives with surrounding whitespace, and then one " +
      "surface stops matching another with nothing thrown and nothing logged.",
  );
  process.exit(1);
}
