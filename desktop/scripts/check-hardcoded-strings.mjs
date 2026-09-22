/**
 * PR6 static gate (spec BUZZ-DESKTOP-I18N-ZH-HANS-001, NFR-007): catch new
 * hardcoded user-facing English in the desktop UI.
 *
 * Why an AST instead of line regexes: the first draft of this file matched
 * `>text<` and `prop="text"` with regular expressions. That missed conditional
 * children, template literals, concatenations, `toast.*` messages and anything
 * inside a JSX comment, and it could not tell a `placeholder` from a
 * `className`. The guard now parses each production source file with the
 * TypeScript parser (a desktop devDependency) and reports only string values
 * that sit in a position which renders as UI copy.
 *
 * Candidate positions (a "candidate" = a literal that must be justified):
 *   - JSX text children, incl. `{"Literal"}`, `{cond ? "A" : "B"}`,
 *     `{cond && "text"}` and the children of Dialog / Tooltip / AlertDialog /
 *     Popover / Sheet elements.
 *   - Static values of copy-bearing JSX props: placeholder, title, aria-label,
 *     aria-labelledby, label, alt, tooltip, description, legend, help text,
 *     confirm/cancel labels, empty/error text, …
 *   - First string argument of `toast` / `toast.error|success|warning|info|…`,
 *     `alert`, `confirm`, `prompt`, `notify` (also the `window.` forms).
 *   - Initializers of copy-bearing object properties / consts (`label`,
 *     `titleText`, `emptyMessage`, …).
 *   - Single-line concatenations and template literals in any position above;
 *     interpolated parts are normalised to `{}` so the baseline key survives a
 *     refactor of the interpolated expression.
 *
 * Never a candidate (structural exclusions — `shouldScanFile`,
 * `findCommentRanges`, `isExcludedValue`, `flattenValue`): test / spec /
 * test-support files, fixture directories, `src/testing`, `src/locales`,
 * `src/i18n`, generated route trees, comments (JS, JSDoc and JSX), arguments of
 * `t()` / logger calls, comparison and other data positions, `className` /
 * `class` / `style` / `id` / `key` / `data-*` / `src` / `href` / `variant` /
 * `type` / `role` / `name` props, code elements (`pre`, `code`, `kbd`, `samp`,
 * `script`, `style`), hex colours, CSS fragments, URLs, emails, paths, npm
 * specifiers, locale tags, protocol and Nostr identifiers (npub/nsec/kind /
 * snake_case payload keys), SCREAMING_SNAKE enum values and brand-only strings
 * (Buzz, Nostr, Tauri, …).
 *
 * Every remaining candidate must appear in hardcoded-strings-baseline.json with
 * a category from the fixed vocabulary below AND a note. Categories are data,
 * not vibes: an unknown category, a missing note, a malformed or duplicate
 * entry is a hard error (exit 2), and a `true-violation` entry is a hard
 * failure (exit 1) that records debt which still has to be fixed. There is no
 * directory-level, prefix-level or wildcard allowlist: an entry matches exactly
 * one file + one candidate kind + one literal, so a new file can never be
 * covered by an entry written for another file.
 *
 * Baseline shape — one record per literal, grouped only to avoid repeating the
 * same file/surface reason 200 times:
 *   { "entries": [ { "file": "src/...", "category": "...", "note": "...",
 *                    "items": [ { "kind": "jsx-text", "literal": "Delete",
 *                                 "note": "per-string reason (required unless the
 *                                 category is a surface deferral)" } ] } ] }
 *
 * Baseline key: `<file>|<kind>|<literal>` — deliberately line-free, because the
 * concurrent localization waves reflow source lines constantly.
 *
 * Exit codes:
 *   0  every candidate is baselined and no entry is a `true-violation`
 *   1  new unbaselined candidate(s), and/or `true-violation` entries
 *   2  the gate itself is broken (missing src dir, unparsable baseline, unknown
 *      category, unparsable source file) — never treated as a pass
 *
 * Usage (from `desktop/`):
 *   node scripts/check-hardcoded-strings.mjs              # the CI gate
 *   node scripts/check-hardcoded-strings.mjs --report     # dump all candidates
 *   node scripts/check-hardcoded-strings.mjs --report-unbaselined
 *
 * Stale entries (baselined, no longer produced by the scan) are advisory, not
 * fatal: waves in flight delete strings faster than this branch re-reviews the
 * baseline, and a stale entry can never admit a new string. Prune them when the
 * owning surface is closed out.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const SCRIPT_PATH = fileURLToPath(import.meta.url);
const SCRIPT_DIR = path.dirname(SCRIPT_PATH);
/** `desktop/` — resolved from this file, so the gate is cwd-independent. */
export const PROJECT_ROOT = path.resolve(SCRIPT_DIR, "..");
const SRC_DIR = path.join(PROJECT_ROOT, "src");
export const BASELINE_PATH = path.join(
  SCRIPT_DIR,
  "hardcoded-strings-baseline.json",
);

/* -------------------------------------------------------------------------- */
/* Baseline vocabulary                                                        */
/* -------------------------------------------------------------------------- */

/**
 * The fixed set of reasons a hardcoded English literal may stay. Every baseline
 * entry must use one of these keys; anything else fails the gate.
 */
export const BASELINE_CATEGORIES = Object.freeze({
  "non-ui-identifier":
    "Sits in a copy position but is an identifier/technical token (enum value, event or route name, cache key), not prose.",
  "test-fixture":
    "Sample/mock/seed data that only surfaces in tests, the e2e bridge or demo fixtures.",
  "protocol-field":
    "Nostr / ACP / relay wire vocabulary: field names, payload keys, kind labels, relay or NIP identifiers.",
  "brand-name":
    "Product, vendor or model name the glossary freezes in English (Buzz, Nostr, Tauri, GitHub, Codex, Claude, model ids).",
  url: "A URL, deep link, wss:// relay address or path shown verbatim.",
  "class-name":
    "Tailwind/class/style value or CSS-ish token that reached a copy-bearing position.",
  "code-sample":
    "Code, command, JSON, clipboard markup or shell output quoted for developers or security notices.",
  "user-content":
    "Literal standing in for user/agent-authored content (display names, channel names, message bodies) that FR-010 forbids translating.",
  "deferred-pr4-surface":
    "Owned by a localization surface that is still in flight (PR3 wave 4 or any PR4 surface); noted so the gate stays green for work in progress. The owning wave must re-triage its own files.",
  "deferred-wave-residual":
    "Real untranslated UI copy the PR6 scanner found in a surface whose wave already merged (sidebar / channels / messages / onboarding). It is owed debt, tracked for the pending PR3 wrap-up: not marked `true-violation` only because PR6 must not fail the gate on files another wave owns.",
  "true-violation":
    "Genuine untranslated UI copy in a finished surface. Always fails the gate until it moves into the catalog.",
});

/** Categories that make the gate exit non-zero. */
export const FAILING_CATEGORIES = Object.freeze(new Set(["true-violation"]));

/* -------------------------------------------------------------------------- */
/* File selection                                                             */
/* -------------------------------------------------------------------------- */

const SCAN_EXTENSIONS = new Set([".ts", ".tsx"]);

const SKIP_DIR_NAMES = new Set([
  "node_modules",
  "dist",
  "build",
  "coverage",
  "test-results",
  "playwright-report",
  "fixtures",
  "__fixtures__",
  "__tests__",
  "__mocks__",
  "__snapshots__",
  "snapshots",
]);

/** POSIX paths relative to `desktop/` that are never scanned. */
const SKIP_PATH_PREFIXES = ["src/locales", "src/i18n", "src/testing"];

const SKIP_FILE_RE = /(\.test|\.spec|\.test-support)\.[cm]?[jt]sx?$/;

/** Mock/fixture modules that live inside production directories. */
const FIXTURE_FILE_RE =
  /(^|\/)(mocks?|fixtures?|samples?|seed|demo-data|test-support)(\.|\/)/i;

export function shouldScanFile(relPath) {
  const posix = relPath.replace(/\\/g, "/");
  const base = path.posix.basename(posix);
  if (!SCAN_EXTENSIONS.has(path.posix.extname(base))) return false;
  if (base.endsWith(".d.ts") || base.endsWith(".gen.ts")) return false;
  if (SKIP_FILE_RE.test(base)) return false;
  // Fixture/mock/seed modules are test material even when they live inside a
  // production feature directory (spec §PR6: exclude test fixtures).
  if (FIXTURE_FILE_RE.test(posix)) return false;
  if (SKIP_PATH_PREFIXES.some((p) => posix === p || posix.startsWith(`${p}/`)))
    return false;
  if (posix.split("/").some((s) => SKIP_DIR_NAMES.has(s))) return false;
  return true;
}

/**
 * Surfaces whose English is still owned by an in-flight localization wave, so a
 * leftover there is `deferred-pr4-surface` rather than a violation. Enumerated
 * on purpose instead of inverted: a brand-new directory is NOT silently
 * deferred — it has to be added here with its owner, or be triaged for real.
 * Every dir listed here is still deliberately English on this branch (spec §7
 * PR3 wave 4 and PR4). Merged waves 1-3 (onboarding, sidebar, channels,
 * messages, settings.appearance) are absent, so leftovers in those files must
 * carry a real non-deferral reason.
 */
export const DEFERRED_SURFACES = Object.freeze([
  { dir: "src/features/notifications", owner: "PR3 wave 4 (notifications)" },
  { dir: "src/features/search", owner: "PR3 wave 4 (search)" },
  { dir: "src/features/home", owner: "PR3 wave 4 (home)" },
  { dir: "src/features/agents", owner: "PR4 (agents)" },
  { dir: "src/features/settings", owner: "PR4 (settings sections)" },
  { dir: "src/features/projects", owner: "PR4 (projects/repos/issues/PRs)" },
  { dir: "src/features/workflows", owner: "PR4 (workflows)" },
  { dir: "src/features/profile", owner: "PR4 (profile)" },
  { dir: "src/features/moderation", owner: "PR4 (moderation)" },
  { dir: "src/features/channel-templates", owner: "PR4 (templates)" },
  { dir: "src/features/terminal", owner: "PR4 (terminal)" },
  { dir: "src/features/huddle", owner: "PR4 (huddle)" },
  { dir: "src/features/chat", owner: "PR4 long tail (chat)" },
  { dir: "src/features/communities", owner: "PR4 long tail (communities)" },
  {
    dir: "src/features/community-members",
    owner: "PR4 long tail (community members)",
  },
  { dir: "src/features/presence", owner: "PR4 long tail (presence)" },
  { dir: "src/features/user-status", owner: "PR4 long tail (user status)" },
  { dir: "src/features/custom-emoji", owner: "PR4 long tail (custom emoji)" },
  { dir: "src/features/gifs", owner: "PR4 long tail (gifs)" },
  { dir: "src/features/pulse", owner: "PR4 long tail (pulse)" },
  { dir: "src/features/reminders", owner: "PR4 long tail (reminders)" },
  { dir: "src/features/mesh-compute", owner: "PR4 long tail (mesh compute)" },
  { dir: "src/features/local-archive", owner: "PR4 long tail (local archive)" },
  {
    dir: "src/features/identity-archive",
    owner: "PR4 long tail (identity archive)",
  },
  { dir: "src/features/agent-memory", owner: "PR4 long tail (agent memory)" },
  // Wave 3 touched only `forum/ui/ForumComposer.lifecycle.test.mjs`; the forum
  // views themselves have never been extracted.
  { dir: "src/features/forum", owner: "PR4 long tail (forum views)" },
  // Shared primitives carry no `t()` calls at all; their chrome text is
  // extracted together with the surfaces that adopt them (PR4).
  { dir: "src/shared/ui", owner: "PR4 (shared UI primitives)" },
  { dir: "src/shared/layout", owner: "PR4 (shared layout chrome)" },
  {
    dir: "src/shared",
    owner: "PR4 (shared lib) / PR5 (formatters, datetime)",
  },
  { dir: "src/protectedFeatures", owner: "PR4 (protected feature chrome)" },
  { dir: "src/app", owner: "PR4 (app shell chrome)" },
]);

export function deferredOwner(relPath) {
  const posix = relPath.replace(/\\/g, "/");
  const hit = DEFERRED_SURFACES.find(
    (s) => posix === s.dir || posix.startsWith(`${s.dir}/`),
  );
  return hit ? hit.owner : null;
}

/* -------------------------------------------------------------------------- */
/* Value classification                                                       */
/* -------------------------------------------------------------------------- */

const CJK_RE = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
const LETTER_RE = /\p{L}/u;

/** Words that carry no translatable meaning on their own. */
const BRAND_WORDS = new Set([
  "buzz",
  "nostr",
  "tauri",
  "github",
  "gitlab",
  "codex",
  "claude",
  "openai",
  "anthropic",
  "macos",
  "ios",
  "android",
  "linux",
  "windows",
  "acp",
  "mcp",
  "nip",
  "url",
  "urls",
  "uri",
  "api",
  "apis",
  "json",
  "jsonrpc",
  "http",
  "https",
  "wss",
  "css",
  "html",
  "md",
  "pr",
  "prs",
  "ui",
  "ux",
  "id",
  "ids",
  "ok",
  "vs",
  "eta",
]);

/** JSX attribute names whose value is UI copy. */
const COPY_ATTRIBUTE_RE =
  /^(aria-label|aria-labelledby|aria-description|arialabel|placeholder|title|tooltip|label|alt|legend|description|help-?text|helptext|confirm(?:ation)?-?(?:label|text|title)|cancel-?(?:label|text)|heading|headline|caption|empty-?(?:state|text|title|message)|error-?(?:text|message|title)|success-?(?:text|message)|button-?(?:text|label)|action-?(?:text|label)|hint|summary|cta)$/i;

/**
 * Attribute names that look copy-bearing but never hold prose. Only consulted
 * for names `COPY_ATTRIBUTE_RE` already accepted, and deliberately free of any
 * `aria-*` veto — `aria-label` is a required detection target.
 */
const NEVER_ATTRIBUTE_RE =
  /^(class|classname|class-name|style|id|key|ref|testid|data-testid|data-.+|role|src|srcset|href|to|target|rel|type|name|value|defaultValue|variant|size|color|colour|background|border|padding|margin|gap|rounded|position|zindex|z-index|flex|grid|weight|shadow|transition|animation|duration|delay|icon|iconname|icon-name|shape|scheme|scheme-name|scheme-slug|scheme-identifier|bundle-id|identifier|version|width|height|as|slot|aschild|dir|lang|htmlfor|for|accept|capture|wrap|part|exportparts|inputmode|autocomplete|crossorigin|referrerpolicy|loading|fetchpriority|decoding|kind|media|sizes|srcdoc|pattern|dirname|enctype|form|formaction|ismap|usemap|span|cols|rows|colspan|rowspan|start|step|multiple|checked|selected|disabled|readonly|required|autofocus|auto-focus|tabindex|contenteditable|draggable|spellcheck|autosave|nonce|csp|accesskey|itemid|itemprop|property|about|datatype|inlist|prefix|resource|typeof|vocab|fill|stroke|viewbox|points|transform|clip-path|mask|filter|opacity|offset|stop-color|font|font-family|font-size|letter-spacing|line-height|x|y|x1|x2|y1|y2|cx|cy|rx|ry|r|dx|dy|k|path|route|routes|pathname|url|link|link-to|component|render|children|of|in|on|by|at|use|from|test|match|matcher|debugid|event|protocol|schemeid)$/i;

/** Object property names whose value is UI copy. */
const COPY_PROPERTY_RE =
  /^(?:[a-z][a-z0-9]*)?(label|title|text|message|tooltip|placeholder|description|heading|caption|hint|summary|legend|body|copy|confirm|cancel|cta)(?:[A-Z][a-zA-Z0-9]*)?$/;

/** `const confirmTitle = …`, `const deleteLabel = …` */
const COPY_VARIABLE_RE =
  /^(?:[a-z][a-zA-Z0-9]*)?(label|title|text|message|tooltip|placeholder|description|heading|caption|hint|body|copy|headline|confirm|cancel)(?:[A-Z][a-zA-Z0-9]*)?$/;

const CLASS_NAME_SUFFIX_RE =
  /(?:class|classes|classname|classnames|style|styles)$/i;

const CODE_ELEMENT_RE = /^(pre|code|kbd|samp|script|style)$/i;

/** Calls whose first string argument is shown to the user. */
const UI_CALL_RE =
  /^(?:window\.|globalThis\.)?(toast|notify|alert|confirm|prompt|showToast|pushToast|openToast|toastError|toastSuccess|toastMessage)$/;
const TOAST_METHODS = new Set([
  "error",
  "success",
  "warning",
  "warn",
  "info",
  "message",
  "loading",
]);
const TOAST_OBJECT_RE = /^(toast|notify)$/;

/** `t()`, `i18n.t()`, `api.t()` — their arguments are catalog keys, not copy. */
const TRANSLATE_CALLEE_RE = /^(?:[a-z][\w$]*\.)*(?:t|tx|translate|text)$/;
/** console/logger/metrics — their arguments are developer text. */
const LOGGER_CALLEE_RE =
  /^(?:console\.\w+|(?:[a-z][\w$]*\.)*(?:logger|log|debug|trace|telemetry|metrics|analytics|sentry|assert|invariant)(?:\.\w+)?)$/;

const ALWAYS_SKIP_RES = [
  /^https?:\/\//i, // URL
  /^www\./i,
  /^[a-z][a-z0-9+.-]*:\/\/\S*$/i, // any scheme (wss://, buzz://, file://, mailto:)
  /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/, // email
  /^([.]{0,2}[\\/]|[a-zA-Z]:[\\/]|\/)\S*$/, // path
  /^#[0-9a-f]{3,8}$/i, // hex colour
  /^(rgba?|hsla?|lab|lch|oklch|color|linear-gradient|radial-gradient|url)\(/i,
  /^v?\d+(\.\d+){1,3}([-+]\S*)?$/, // semver
  /^\d+(\.\d+)?(px|rem|em|ch|%|vh|vw|ms|s|deg|x)?$/i, // dimension
  /^[a-z]{2}(?:-[A-Za-z]{2,4})?(?:-[A-Za-z]{2})?$/, // bcp47-ish locale
  /^@?[a-z0-9][a-z0-9._-]*(?:\/[a-z0-9._@-]+)?$/, // npm-ish bare specifier
  /^\/[\w*/:{}.[\]|+-]+\/[a-z]*$/, // regex source as a string
];

/** Wire identifiers / payload keys that survive the copy shape test. */
const PROTOCOLISH_RES = [
  /^(?:kind|event|tag|content|sig|id|pubkey|created_at|author|tags)\s*[:=]\s*\S*$/i,
  /^nostr:/i,
  /^n(pub|sec|pubkey|profile|event|addr|encrypt(ed)?)[a-z0-9]+$/i,
  /^[0-9a-f]{6,}$/i, // hex ids
  /^[a-z][a-z0-9]*(?:_[a-z0-9]+)+$/, // snake_case payload key
  /^[a-z]+(?:-[a-z0-9]+){1,}$/, // kebab identifier, e.g. image-png
  /^[A-Z][A-Z0-9_]*$/, // SCREAMING_SNAKE
  /^[A-Z]{2,}(?:[-_][A-Z0-9]{2,})*$/, // all-caps token(s), e.g. UTC, NPA
  /^[a-z]+(?:\.[a-z0-9_-]+)+$/, // dotted config key
  /^[a-z]+(?:[A-Z][a-z0-9]+)+$/, // camelCase identifier
  /\{\{\s*[\w.]+\s*\}\}\s*$/u, // ends in a bare interpolation, e.g. "{{count}}"
];

/** True when the literal can never be UI copy. */
export function isExcludedValue(value) {
  if (value.length < 2) return true;
  if (!LETTER_RE.test(value)) return true; // numbers / punctuation only
  if (CJK_RE.test(value)) return true; // already translated
  const collapsed = value.replace(/\s+/g, " ").trim();
  if (collapsed.length < 2) return true;
  if (ALWAYS_SKIP_RES.some((re) => re.test(collapsed))) return true;
  if (PROTOCOLISH_RES.some((re) => re.test(collapsed))) return true;
  if (/^[a-z-]+\s*:\s*[^;{}]+;?$/.test(collapsed)) return true; // CSS declaration
  if (/\[[^\]]*(px|rem|vh|vw|%)[^\]]*\]/.test(collapsed)) return true; // tw arbitrary
  // `{}` is the marker this scanner substitutes for an interpolated expression,
  // so it must not be mistaken for code or markup.
  const prose = collapsed.replace(/\{\}/g, " ").trim();
  if (/[{}<>`;\\]|=>|\$/.test(prose)) return true; // code / markup
  if (/^[A-Za-z][\w.]*\([^)]*\)$/.test(prose)) return true; // fn call shape
  const words = collapsed.split(/[\s/]+/).filter(Boolean);
  if (words.length === 0) return true;
  if (words.every((w) => isBrandOrAcronym(w))) return true; // "Buzz", "Nostr PR"
  // A single lowercase word ("submit", "npub", "cancel") is an identifier or a
  // value in every position this scanner looks at; capitalised words are copy.
  if (
    words.length === 1 &&
    !/^[A-Z]/.test(collapsed) &&
    !/[.!?]$/.test(collapsed)
  )
    return true;
  return false;
}

function isBrandOrAcronym(word) {
  const stripped = word.replace(/[.,!?;:]/g, "");
  if (BRAND_WORDS.has(stripped.toLowerCase())) return true;
  return /^[A-Z]{2,6}s?$/.test(stripped);
}

/** Positive test for prose-shaped UI copy. */
export function looksLikeCopy(value) {
  const collapsed = value.replace(/\s+/g, " ").trim();
  if (collapsed.length < 2) return false;
  if (!/[A-Za-z]/.test(collapsed)) return false;
  if (/[A-Za-z]{2,}\s+[A-Za-z]{2,}/.test(collapsed)) return true; // "Sign out"
  if (/^[A-Z][A-Za-z]*$/.test(collapsed)) return true; // "Delete", "Loading"
  return /^[A-Z]/.test(collapsed) || /[.!?]$/.test(collapsed);
}

/* -------------------------------------------------------------------------- */
/* Extraction                                                                 */
/* -------------------------------------------------------------------------- */

/** Every comment span in a file, from TypeScript's own lexer. */
export function findCommentRanges(text) {
  const scanner = ts.createScanner(
    ts.ScriptTarget.Latest,
    /* skipTrivia */ false,
    ts.LanguageVariant.Standard,
    text,
  );
  const ranges = [];
  let token = scanner.scan();
  let guard = 0;
  while (token !== ts.SyntaxKind.EndOfFileToken && guard++ < 5_000_000) {
    if (
      token === ts.SyntaxKind.SingleLineCommentTrivia ||
      token === ts.SyntaxKind.MultiLineCommentTrivia
    ) {
      ranges.push([scanner.getTokenPos(), scanner.getTextPos()]);
    }
    token = scanner.scan();
  }
  return ranges;
}

function inComments(pos, ranges) {
  for (const [start, end] of ranges) {
    if (start > pos) break;
    if (pos >= start && pos < end) return true;
  }
  return false;
}

function stripQuotes(s) {
  return s.replace(/^["']|["']$/g, "");
}

function attributeText(name) {
  const raw = ts.isJsxNamespacedName(name)
    ? `${name.namespace.getText()}:${name.name.getText()}`
    : name.getText();
  return stripQuotes(raw);
}

function calleeText(expr) {
  if (!expr) return "";
  if (ts.isIdentifier(expr)) return expr.text;
  if (ts.isPropertyAccessExpression(expr)) {
    const inner = calleeText(expr.expression);
    return inner ? `${inner}.${expr.name.text}` : expr.name.text;
  }
  if (ts.isCallExpression(expr)) return calleeText(expr.expression);
  if (expr.kind === ts.SyntaxKind.ThisKeyword) return "this";
  return "";
}

function isTranslateCall(dotted) {
  if (!dotted) return false;
  const tail = dotted.split(".").pop() ?? "";
  if (!TRANSLATE_CALLEE_RE.test(dotted)) return false;
  // `t(`, `i18n.t(`, `api.t(` count; `setTimeout(`, `getToken(` do not.
  return (
    tail === "t" || tail === "tx" || tail === "translate" || tail === "text"
  );
}

function isLoggerCall(dotted) {
  return dotted ? LOGGER_CALLEE_RE.test(dotted) : false;
}

function isUiCall(dotted) {
  if (!dotted) return false;
  if (UI_CALL_RE.test(dotted)) return true;
  const parts = dotted.split(".");
  return (
    parts.length === 2 &&
    TOAST_OBJECT_RE.test(parts[0]) &&
    TOAST_METHODS.has(parts[1])
  );
}

function normalizeLiteral(raw) {
  return raw.replace(/\s+/g, " ").trim();
}

function stripGlobal(dotted) {
  return dotted.replace(/^window\./, "").replace(/^globalThis\./, "");
}

/**
 * Flatten one value expression into literal text. String literals and
 * substitution-free templates contribute their text; template spans and
 * non-literal operands contribute `{}`; single-line `+` chains, `&&` and `??`
 * are flattened into one string. Returns whether a real literal was found.
 */
function flattenValue(node, acc, depth = 0) {
  if (!node || depth > 8) return false;
  if (ts.isParenthesizedExpression(node))
    return flattenValue(node.expression, acc, depth + 1);
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
    acc.push(node.text);
    return true;
  }
  if (node.kind === ts.SyntaxKind.TemplateExpression) {
    acc.push(node.head.text);
    for (const span of node.templateSpans) {
      acc.push("{}");
      if (span.literal && typeof span.literal.text === "string")
        acc.push(span.literal.text);
    }
    return true;
  }
  if (ts.isBinaryExpression(node)) {
    const kind = node.operatorToken.kind;
    if (kind === ts.SyntaxKind.PlusToken) {
      const left = flattenValue(node.left, acc, depth + 1);
      const right = flattenValue(node.right, acc, depth + 1);
      return left || right;
    }
    if (
      kind === ts.SyntaxKind.AmpersandAmpersandToken ||
      kind === ts.SyntaxKind.QuestionQuestionToken
    )
      return flattenValue(node.right, acc, depth + 1);
    return false;
  }
  acc.push("{}");
  return false;
}

/**
 * Candidates for one already-parsed source file.
 *
 * @returns {Array<{file:string,line:number,column:number,kind:string,literal:string,occurrences:number}>}
 */
export function extractCandidates(sourceFile, relPath) {
  const comments = findCommentRanges(sourceFile.getFullText());
  const found = new Map();

  const lineOf = (pos) =>
    sourceFile.getLineAndCharacterOfPosition(pos).line + 1;
  const columnOf = (pos) =>
    sourceFile.getLineAndCharacterOfPosition(pos).character + 1;

  function record(node, kind, rawLiteral) {
    const literal = normalizeLiteral(rawLiteral ?? "");
    if (!literal) return;
    if (isExcludedValue(literal)) return;
    if (!looksLikeCopy(literal)) return;
    // A JsxText node spans the surrounding indentation, so report the line its
    // text actually starts on.
    const isText = ts.isJsxText(node);
    const leading = isText
      ? (node.text.match(/^\s*/)?.[0].length ?? 0)
      : node.getLeadingTriviaWidth(sourceFile);
    const pos = node.getStart(sourceFile) + leading;
    if (inComments(node.getStart(sourceFile), comments)) return;
    // Multi-line concatenations and template literals usually hold structured
    // content; the spec scopes that detection to single-line strings. Multi-line
    // JSX text is the normal shape of a dialog description, so it stays.
    if (!isText && lineOf(node.getEnd(sourceFile)) !== lineOf(pos)) return;
    const key = `${kind}|${literal}`;
    const existing = found.get(key);
    if (existing) {
      existing.occurrences += 1;
      return;
    }
    found.set(key, {
      file: relPath,
      line: lineOf(pos),
      column: columnOf(pos),
      kind,
      literal,
      occurrences: 1,
    });
  }

  /** Every literal branch of a value expression becomes its own candidate. */
  function collectFromValue(node, kind, depth = 0) {
    if (!node || depth > 8) return;
    if (ts.isParenthesizedExpression(node))
      return collectFromValue(node.expression, kind, depth + 1);
    if (ts.isConditionalExpression(node)) {
      collectFromValue(node.whenTrue, kind, depth + 1);
      collectFromValue(node.whenFalse, kind, depth + 1);
      return;
    }
    const acc = [];
    if (flattenValue(node, acc)) record(node, kind, acc.join(""));
  }

  function isJsxChildrenPosition(node) {
    const parent = node.parent;
    return Boolean(
      parent && (ts.isJsxElement(parent) || ts.isJsxFragment(parent)),
    );
  }

  function insideCodeElement(node) {
    let cur = node.parent;
    while (cur) {
      if (
        ts.isJsxElement(cur) &&
        CODE_ELEMENT_RE.test(cur.openingElement.tagName.getText(sourceFile))
      )
        return true;
      cur = cur.parent;
    }
    return false;
  }

  function visit(node, inert) {
    if (ts.isJsxText(node)) {
      if (!insideCodeElement(node)) record(node, "jsx-text", node.text);
      return;
    }

    if (ts.isJsxAttribute(node)) {
      const name = attributeText(node.name);
      if (
        COPY_ATTRIBUTE_RE.test(name) &&
        !NEVER_ATTRIBUTE_RE.test(name) &&
        node.initializer
      ) {
        const value = ts.isJsxExpression(node.initializer)
          ? node.initializer.expression
          : node.initializer;
        if (!inert || isUiCall(calleeText(value)))
          collectFromValue(value, `attr:${name}`);
      }
      ts.forEachChild(node, (child) => visit(child, inert));
      return;
    }

    if (ts.isJsxExpression(node) && isJsxChildrenPosition(node)) {
      const value = node.expression;
      if (!inert || isUiCall(calleeText(value)))
        collectFromValue(value, "jsx-expr");
      ts.forEachChild(node, (child) => visit(child, inert));
      return;
    }

    if (ts.isCallExpression(node)) {
      const dotted = calleeText(node.expression);
      const uiCall = isUiCall(dotted);
      const childInert =
        inert || isTranslateCall(dotted) || isLoggerCall(dotted);
      if (uiCall && !inert && node.arguments.length > 0)
        collectFromValue(node.arguments[0], `call:${stripGlobal(dotted)}`);
      ts.forEachChild(node, (child) => {
        if (child === node.expression) return;
        visit(child, childInert);
      });
      return;
    }

    if (ts.isPropertyAssignment(node)) {
      const name = stripQuotes(node.name.getText(sourceFile));
      if (
        !inert &&
        COPY_PROPERTY_RE.test(name) &&
        !CLASS_NAME_SUFFIX_RE.test(name) &&
        !NEVER_ATTRIBUTE_RE.test(name)
      )
        collectFromValue(node.initializer, `prop:${name}`);
      ts.forEachChild(node, (child) => visit(child, inert));
      return;
    }

    if (ts.isVariableDeclaration(node) && node.initializer) {
      const name = node.name.getText(sourceFile);
      if (
        !inert &&
        COPY_VARIABLE_RE.test(name) &&
        !CLASS_NAME_SUFFIX_RE.test(name)
      )
        collectFromValue(node.initializer, `const:${name}`);
      ts.forEachChild(node, (child) => visit(child, inert));
      return;
    }

    ts.forEachChild(node, (child) => visit(child, inert));
  }

  visit(sourceFile, false);
  return [...found.values()].sort(
    (a, b) => a.line - b.line || a.kind.localeCompare(b.kind),
  );
}

/* -------------------------------------------------------------------------- */
/* Corpus walk                                                                */
/* -------------------------------------------------------------------------- */

export function parseSourceFile(absPath, text) {
  return ts.createSourceFile(
    path.basename(absPath),
    text,
    ts.ScriptTarget.Latest,
    /* setParentNodes */ true,
    absPath.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
}

/** Parse + extract for one snippet (used by the unit test and `--report` peers). */
export function scanSourceText(text, relPath = "src/sample/Sample.tsx") {
  return extractCandidates(parseSourceFile(relPath, text), relPath);
}

function* walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) =>
    a.name.localeCompare(b.name),
  )) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (SKIP_DIR_NAMES.has(entry.name)) continue;
      const rel = path.relative(PROJECT_ROOT, full).replace(/\\/g, "/");
      if (SKIP_PATH_PREFIXES.some((p) => rel === p || rel.startsWith(`${p}/`)))
        continue;
      yield* walk(full);
    } else if (entry.isFile()) {
      yield full;
    }
  }
}

/**
 * Scan the whole production source tree.
 *
 * @returns {{candidates: Array, filesScanned:number, parseFailures:string[]}}
 */
export function scanCorpus({ root = SRC_DIR } = {}) {
  if (!existsSync(root))
    throw new Error(`desktop source directory not found: ${root}`);
  const candidates = [];
  const parseFailures = [];
  let filesScanned = 0;
  for (const abs of walk(root)) {
    const rel = path.relative(PROJECT_ROOT, abs).replace(/\\/g, "/");
    if (!shouldScanFile(rel)) continue;
    filesScanned += 1;
    const sourceFile = parseSourceFile(abs, readFileSync(abs, "utf8"));
    const errors = (sourceFile.parseDiagnostics ?? []).filter(
      (d) => d.category === ts.DiagnosticCategory.Error,
    );
    if (errors.length > 0) {
      // A file we cannot parse is a gate failure, never a silent skip.
      parseFailures.push(
        `${rel}: ${errors.length} parse error(s): ${ts.flattenDiagnosticMessageText(
          errors[0].messageText,
          " ",
        )}`,
      );
      continue;
    }
    for (const c of extractCandidates(sourceFile, rel)) candidates.push(c);
  }
  return { candidates, filesScanned, parseFailures };
}

/* -------------------------------------------------------------------------- */
/* Baseline                                                                   */
/* -------------------------------------------------------------------------- */

export function entryId(entry) {
  return `${entry.file}|${entry.kind}|${entry.literal}`;
}

/**
 * Only a per-surface deferral may lean on the group note. Every other reason is
 * a claim about one specific string, so it has to be written next to that
 * string — that is what stops the baseline becoming a blanket accept.
 */
const GROUP_NOTE_IS_ENOUGH = new Set(["deferred-pr4-surface"]);

/**
 * Validate the raw baseline document and flatten it into one record per
 * literal: `{file, kind, literal, category, note}`.
 */
export function parseBaseline(raw) {
  const errors = [];
  const entries = [];
  if (!raw || !Array.isArray(raw.entries))
    return { entries, errors: ["baseline document has no `entries` array"] };
  const seen = new Set();
  raw.entries.forEach((group, index) => {
    const where = `group #${index + 1}`;
    if (!group || typeof group !== "object") {
      errors.push(`${where}: not an object`);
      return;
    }
    for (const field of ["file", "category", "note"]) {
      if (typeof group[field] !== "string" || group[field].trim() === "") {
        errors.push(`${where}: \`${field}\` must be a non-empty string`);
        return;
      }
    }
    if (!Object.hasOwn(BASELINE_CATEGORIES, group.category)) {
      errors.push(
        `${where}: unknown category \`${group.category}\`; allowed: ${Object.keys(
          BASELINE_CATEGORIES,
        ).join(", ")}`,
      );
      return;
    }
    if (!Array.isArray(group.items) || group.items.length === 0) {
      errors.push(`${where}: \`items\` must be a non-empty array`);
      return;
    }
    if (typeof group.file !== "string" || !/^src\//.test(group.file)) {
      errors.push(
        `${where}: \`file\` must be a src/... path relative to desktop/`,
      );
      return;
    }
    group.items.forEach((item, itemIndex) => {
      const at = `${where} item #${itemIndex + 1}`;
      if (!item || typeof item !== "object") {
        errors.push(`${at}: not an object`);
        return;
      }
      for (const field of ["kind", "literal"]) {
        if (typeof item[field] !== "string" || item[field].trim() === "") {
          errors.push(`${at}: \`${field}\` must be a non-empty string`);
          return;
        }
      }
      const needsOwnNote = !GROUP_NOTE_IS_ENOUGH.has(group.category);
      const note =
        typeof item.note === "string" && item.note.trim()
          ? item.note.trim()
          : group.note.trim();
      if (
        needsOwnNote &&
        !(typeof item.note === "string" && item.note.trim())
      ) {
        errors.push(
          `${at}: category \`${group.category}\` requires its own note on the literal`,
        );
        return;
      }
      const record = {
        file: group.file,
        kind: item.kind,
        literal: item.literal,
        category: group.category,
        note,
      };
      const id = entryId(record);
      if (seen.has(id)) {
        errors.push(`${at}: duplicate entry ${id}`);
        return;
      }
      seen.add(id);
      entries.push(record);
    });
  });
  return { entries, errors };
}

export function loadBaseline(file = BASELINE_PATH) {
  if (!existsSync(file)) throw new Error(`missing baseline file: ${file}`);
  let doc;
  try {
    doc = JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    throw new Error(`baseline at ${file} is not valid JSON: ${error.message}`);
  }
  const { entries, errors } = parseBaseline(doc);
  if (errors.length > 0)
    throw new Error(
      `invalid baseline at ${file}:\n  - ${errors.slice(0, 20).join("\n  - ")}`,
    );
  return entries;
}

/* -------------------------------------------------------------------------- */
/* Gate                                                                       */
/* -------------------------------------------------------------------------- */

/**
 * Compare candidates against the baseline. Pure, so the unit test can falsify
 * it without touching the repository.
 */
export function evaluateGate({ candidates, entries }) {
  const byId = new Map(entries.map((e) => [entryId(e), e]));
  const candidateIds = new Set(candidates.map((c) => entryId(c)));
  const unbaselined = candidates.filter((c) => !byId.has(entryId(c)));
  const violations = entries.filter(
    (e) => FAILING_CATEGORIES.has(e.category) && candidateIds.has(entryId(e)),
  );
  const stale = entries.filter((e) => !candidateIds.has(entryId(e)));
  return {
    ok: unbaselined.length === 0 && violations.length === 0,
    unbaselined,
    violations,
    stale,
    candidateCount: candidates.length,
    baselineSize: entries.length,
  };
}

function formatCandidate(c) {
  return `  ${c.file}:${c.line}  [${c.kind}]  ${JSON.stringify(c.literal)}`;
}

export function main({
  argv = process.argv.slice(2),
  write = (s) => process.stdout.write(`${s}\n`),
  writeErr = (s) => process.stderr.write(`${s}\n`),
} = {}) {
  const flags = new Set(argv);
  let scan;
  try {
    scan = scanCorpus();
  } catch (error) {
    writeErr(`check-hardcoded-strings: ${error.message}`);
    return 2;
  }

  if (flags.has("--report") || flags.has("--report-unbaselined")) {
    let rows = scan.candidates;
    if (flags.has("--report-unbaselined")) {
      let entries;
      try {
        entries = loadBaseline();
      } catch (error) {
        writeErr(`check-hardcoded-strings: ${error.message}`);
        return 2;
      }
      rows = evaluateGate({ candidates: scan.candidates, entries }).unbaselined;
    }
    write(JSON.stringify(rows, null, 2));
    return 0;
  }

  let entries;
  try {
    entries = loadBaseline();
  } catch (error) {
    writeErr(`check-hardcoded-strings: ${error.message}`);
    return 2;
  }

  const result = evaluateGate({ candidates: scan.candidates, entries });

  if (scan.parseFailures.length > 0) {
    writeErr(
      `check-hardcoded-strings: ${scan.parseFailures.length} file(s) could not be parsed (counted as gate failures, not skipped):`,
    );
    for (const f of scan.parseFailures.slice(0, 10)) writeErr(`  ${f}`);
    return 2;
  }

  if (result.violations.length > 0) {
    writeErr(
      `check-hardcoded-strings: ${result.violations.length} baselined entr${
        result.violations.length === 1 ? "y is" : "ies are"
      } marked true-violation — the text must be translated:`,
    );
    for (const v of result.violations.slice(0, 40))
      writeErr(`${formatCandidate(v)}  — ${v.note}`);
    if (result.violations.length > 40)
      writeErr(`  … and ${result.violations.length - 40} more`);
  }

  if (result.unbaselined.length > 0) {
    writeErr(
      `check-hardcoded-strings: ${result.unbaselined.length} hardcoded user-facing English string(s) with no baseline entry:`,
    );
    for (const c of result.unbaselined.slice(0, 40))
      writeErr(formatCandidate(c));
    if (result.unbaselined.length > 40)
      writeErr(`  … and ${result.unbaselined.length - 40} more`);
    writeErr(
      "\nFix by moving the text into src/locales/en.json + zh-Hans.json and\n" +
        "rendering it through t(). If the literal is genuinely not UI copy, add an entry\n" +
        "to scripts/hardcoded-strings-baseline.json naming one of the fixed categories\n" +
        "and why. Text still owned by a surface being localized in another wave goes in\n" +
        "as `deferred-pr4-surface` with that wave noted.",
    );
  }

  if (result.stale.length > 0) {
    write(
      `check-hardcoded-strings: ${result.stale.length} baseline entr${
        result.stale.length === 1 ? "y" : "ies"
      } no longer match a scanned candidate (advisory — prune when the owning surface closes):`,
    );
    for (const s of result.stale.slice(0, 15))
      write(`  ${s.file}  [${s.kind}]  ${JSON.stringify(s.literal)}`);
    if (result.stale.length > 15)
      write(`  … and ${result.stale.length - 15} more`);
  }

  const counts = new Map();
  for (const e of entries)
    counts.set(e.category, (counts.get(e.category) ?? 0) + 1);
  const summary =
    `scanned ${scan.filesScanned} files, ${result.candidateCount} candidates; ` +
    `baseline ${result.baselineSize} entries (${[...counts]
      .sort((a, b) => b[1] - a[1])
      .map(([k, v]) => `${k}=${v}`)
      .join(" ")}); ` +
    `unbaselined ${result.unbaselined.length}, true-violations ${result.violations.length}, stale ${result.stale.length}.`;

  if (!result.ok) {
    writeErr(`FAIL — ${summary}`);
    return 1;
  }
  write(`OK — ${summary}`);
  return 0;
}

const invokedDirectly =
  process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(SCRIPT_PATH);
if (invokedDirectly) process.exitCode = main();
