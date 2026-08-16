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

// A quote or backtick immediately followed by an identity namespace, plus the
// first interpolation or word after it. Matching the opening delimiter keeps
// `builtin:fizz`, `"persona"`, and prose containing "persona:" from tripping
// it; capturing what FOLLOWS the namespace is what makes an allowlist entry
// specific to one key rather than to the whole file (see `overrides`).
const IDENTITY_KEY_RE = /[`"'](?:pubkey|persona):(?:\$\{[^}]*\}|[\w.-]*)/g;

/**
 * Allowlisted `relativePath:matchedLiteral` pairs.
 *
 * **Empty on purpose, and worth keeping that way.** The first version of this
 * guard carried four entries and matched only the bare `persona:` prefix, so
 * each entry exempted *every* occurrence of that prefix in the file — and the
 * exempted files were the agent-adjacent ones, i.e. exactly the code most
 * likely to drift. The guard was theatre over its highest-risk surface.
 *
 * The fix was not a tighter allowlist but removing the need for one: the two
 * legitimate non-identity uses now carry their own namespaces
 * (`catalog-persona:` for the persona catalog dialog's selection token,
 * `profile:` for the profile panel's render key), so neither looks like an
 * agent identity to this guard or to a reader.
 *
 * Prefer that route. If an entry is ever genuinely unavoidable, note that the
 * key now includes the text after the namespace, so it scopes to one literal —
 * but a namespace of its own is still the better answer.
 */
const overrides = new Set([]);

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
