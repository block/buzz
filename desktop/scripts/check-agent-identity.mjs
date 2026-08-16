import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Guards the single definition of agent identity.
 *
 * Two surfaces answer "which agents exist" — @-mention autocomplete and the
 * Agents library — and each once hand-rolled its own key. They drifted, and the
 * drift was invisible until an agent became unreachable on one of them:
 * autocomplete keyed on the pubkey (#5202) while the library still grouped by
 * `personaId`, so renaming an instance made it vanish from the library
 * entirely. Nothing failed loudly; a card simply stopped existing.
 *
 * The invariant that prevents a third answer appearing: an agent identity or
 * display-group key is minted in exactly ONE module,
 * `src/features/agents/lib/agentIdentity.ts`. Everywhere else imports it.
 *
 * So this flags a string or template literal that *begins* an identity
 * namespace — `pubkey:` or `persona:` — anywhere outside that module. Those
 * two prefixes are the wire format `agentIdentityKey` and
 * `agentDisplayGroupKey` produce; a literal starting with one is either a
 * second implementation or something close enough to become one.
 *
 * Deliberately narrow. A guard that cries wolf gets disabled, so it does not
 * try to catch every possible way of grouping agents — only the shape that
 * actually caused the outage. Non-identity uses of the same prefix (a dialog's
 * selection token, a render key for a panel with no pubkey) are allowlisted
 * below with a reason each.
 */

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

/** The one module allowed to mint these keys. */
const CANONICAL_MODULE = "src/features/agents/lib/agentIdentity.ts";

const SCAN_ROOT = "src";
const EXTENSIONS = new Set([".ts", ".tsx"]);

// A quote or backtick immediately followed by an identity namespace. Matching
// the opening delimiter is what keeps `builtin:fizz`, `"persona"`, or a
// sentence containing "persona:" from tripping it.
const IDENTITY_KEY_RE = /[`"'](?:pubkey|persona):/g;

// Allowlisted `relativePath:matchedLiteral` pairs. Matching the literal rather
// than a line number keeps these stable when unrelated edits move code.
const overrides = new Set([
  // A radio-group selection token for the persona catalog dialog: it addresses
  // a row in that dialog's own list, is never compared against an agent, and
  // never leaves the component.
  'src/features/agents/ui/PersonaCatalogDialog.tsx:"persona:',
  "src/features/agents/ui/PersonaCatalogDialog.tsx:`persona:",
  // React render key for the profile panel when there is no pubkey to key on
  // (an uninstantiated persona has no agent identity yet). Adjacent to the
  // real thing — if this ever starts being compared against an agent key,
  // route it through `agentIdentity` instead of widening this exception.
  "src/features/profile/ui/UserProfilePanel.tsx:`persona:",
  "src/features/profile/ui/UserProfilePanelUtils.ts:`persona:",
]);

/**
 * Whether a line is wholly a comment. Deliberately a heuristic over the common
 * shapes (`//`, `/*`, and the `*` continuation of a block comment) rather than
 * a parser: a false negative here just means the allowlist earns an entry,
 * while a parser would be far more machinery than this guard is worth.
 */
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
  // Override keys and the canonical path are authored with `/`, but
  // path.relative yields `\` on Windows — compare in posix form or this
  // silently matches nothing.
  const relativePath = path
    .relative(projectRoot, filePath)
    .split(path.sep)
    .join("/");

  if (!EXTENSIONS.has(path.extname(relativePath))) {
    continue;
  }
  if (relativePath === CANONICAL_MODULE) {
    continue;
  }

  const content = await fs.readFile(filePath, "utf8");
  content.split(/\r?\n/).forEach((line, index) => {
    // Prose describing a key format is not a second implementation of one.
    // Unrelated subsystems document their own scope keys (e.g. the channel
    // storage scope `"pubkey:normalizedRelayUrl"`), and flagging a comment
    // teaches people to silence the guard rather than read it.
    if (isCommentLine(line)) {
      return;
    }
    for (const match of line.match(IDENTITY_KEY_RE) ?? []) {
      if (!overrides.has(`${relativePath}:${match}`)) {
        violations.push({ relativePath, lineNumber: index + 1, match });
      }
    }
  });
}

if (violations.length > 0) {
  console.error("Desktop agent-identity check failed:");
  for (const violation of violations) {
    console.error(
      `- ${violation.relativePath}:${violation.lineNumber}: ${violation.match}`,
    );
  }
  console.error(
    `Agent identity is minted in one place: \`${CANONICAL_MODULE}\`. ` +
      "Import `agentIdentityKey` (identity — the pubkey) or " +
      "`agentDisplayGroupKey` (presentation — which agents may share one card) " +
      "instead of building the key here. Two surfaces that answer " +
      '"which agents exist" differently is how a renamed agent silently ' +
      "disappeared from the Agents library. If this literal is genuinely not " +
      "an agent identity, add a narrowly scoped `relativePath:matchedLiteral` " +
      "exception, with a reason, in `desktop/scripts/check-agent-identity.mjs`.",
  );
  process.exit(1);
}
