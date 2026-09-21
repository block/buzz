/**
 * Catalog integrity guard for `desktop/src/locales` (spec §NFR-002, T-005).
 *
 * Rules:
 * - `en` is the source of truth: every leaf key it has must exist in
 *   `zh-Hans`, and `zh-Hans` must not invent keys.
 * - Interpolation variables (`{{name}}`) must match exactly, per key, across
 *   the two catalogs — a renamed or dropped variable breaks the consumer.
 * - `zh-Hans` P0 leaves must not be empty: an empty string would fall back to
 *   English silently and hide missing translations.
 * - No raw HTML in values: translations are data, not markup (spec §4.3).
 *   `<Trans>` component tags are detected separately and compared for parity.
 * - Every `t("a.b.c")` call site under `desktop/src` resolves in **both**
 *   catalogs. Catalog-to-catalog parity cannot catch this one: a key renamed
 *   on the component side only renders the raw dotted key to the user while
 *   every other gate stays green.
 *
 * Used by the unit test (`i18n.test.mjs`); the PR-6 CI gate reuses this
 * module.
 */

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const LOCALES_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "locales",
);
const SRC_DIR = path.resolve(LOCALES_DIR, "..");

/** Read both catalogs fresh from disk. */
export function loadCatalogs() {
  const read = (name) =>
    JSON.parse(readFileSync(path.join(LOCALES_DIR, `${name}.json`), "utf8"));
  return { en: read("en"), zhHans: read("zh-Hans") };
}

/** Flatten a catalog to `a.b.c -> "leaf string"` (objects only). */
export function flattenCatalog(value, prefix = "", out = {}) {
  if (typeof value === "string") {
    out[prefix] = value;
  } else if (value && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      flattenCatalog(child, prefix ? `${prefix}.${key}` : key, out);
    }
  }
  return out;
}

const INTERPOLATION_RE = /\{\{\s*([a-zA-Z_$][\w$]*)/g;

/** Named interpolation variables referenced by one string value. */
export function interpolationVariables(value) {
  const vars = new Set();
  for (const match of value.matchAll(INTERPOLATION_RE)) {
    vars.add(match[1]);
  }
  return vars;
}

/** `<Tag>` component markers used by `<Trans>` translations. */
export function transComponents(value) {
  const tags = new Set();
  for (const match of value.matchAll(/<[A-Za-z][\w.]*[\s>/]/g)) {
    tags.add(match[0].replace(/[\s>/]/g, ""));
  }
  return tags;
}

function symmetricDifference(a, b) {
  const missingInB = [...a].filter((item) => !b.has(item));
  const extraInB = [...b].filter((item) => !a.has(item));
  return [...missingInB, ...extraInB];
}

/**
 * Validate `zh-Hans` against the `en` source catalog.
 * Returns a list of human-readable problems; an empty list means the
 * catalogs are in parity and safe.
 */
export function validateCatalogParity(en, zhHans) {
  const errors = [];
  const enFlat = flattenCatalog(en);
  const zhFlat = flattenCatalog(zhHans);

  for (const [key, enValue] of Object.entries(enFlat)) {
    if (!(key in zhFlat)) {
      errors.push(`missing zh-Hans key: ${key}`);
      continue;
    }
    const zhValue = zhFlat[key];
    if (typeof zhValue !== "string" || zhValue.trim() === "") {
      errors.push(`empty or non-string zh-Hans value: ${key}`);
      continue;
    }
    const varDiff = symmetricDifference(
      interpolationVariables(enValue),
      interpolationVariables(zhValue),
    );
    if (varDiff.length > 0) {
      errors.push(
        `interpolation variable drift on ${key}: ${varDiff.join(", ")}`,
      );
    }
    const tagDiff = symmetricDifference(
      transComponents(enValue),
      transComponents(zhValue),
    );
    if (tagDiff.length > 0) {
      errors.push(`Trans component drift on ${key}: ${tagDiff.join(", ")}`);
    }
    if (/<\/?[A-Za-z]/.test(zhValue)) {
      errors.push(`raw HTML in zh-Hans value: ${key}`);
    }
  }

  for (const key of Object.keys(zhFlat)) {
    if (!(key in enFlat)) {
      errors.push(`unexpected zh-Hans key (not in en source): ${key}`);
    }
  }

  return errors;
}

export const TRANSLATION_CALL_RE = /\bt\(\s*["']([\w.-]+)["']/g;

/**
 * Drop whole-line comments so a key named in prose is not mistaken for a call
 * site, while keeping one output line per input line so line numbers stay
 * true. A trailing comment after code is still scanned; a `t()` call that
 * lives entirely inside a block comment is not.
 */
export function withoutWholeLineComments(text) {
  return text
    .split("\n")
    .map((line) => (/^\s*(\/\/|\*|\/\*)/.test(line) ? "" : line))
    .join("\n");
}

function* sourceFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) yield* sourceFiles(full);
    else if (/\.tsx?$/.test(entry.name)) yield full;
  }
}

/**
 * Every literal `t("a.b.c")` key used by the app source, mapped to its
 * `file:line` call sites. Keys built at runtime are not literals and so are
 * not claimed here.
 */
export function collectTranslationCallSites() {
  const sites = new Map();
  for (const file of sourceFiles(SRC_DIR)) {
    const text = withoutWholeLineComments(readFileSync(file, "utf8"));
    for (const match of text.matchAll(TRANSLATION_CALL_RE)) {
      const line = text.slice(0, match.index).split("\n").length;
      const at = `${path.relative(SRC_DIR, file).replace(/\\/g, "/")}:${line}`;
      const found = sites.get(match[1]);
      if (found) found.push(at);
      else sites.set(match[1], [at]);
    }
  }
  return sites;
}

/**
 * Report every call site whose key resolves in neither catalog (or in only
 * one of them). Empty means the components and the catalogs agree.
 */
export function validateTranslationCallSites(en, zhHans) {
  const enKeys = new Set(Object.keys(flattenCatalog(en)));
  const zhKeys = new Set(Object.keys(flattenCatalog(zhHans)));
  /**
   * A plural call site reads the base key (`t("k", { count })`) while the
   * catalogs store `k_one` + `k_other`. Requiring both forms keeps a
   * half-declared plural failing: one form alone would render the other
   * count as the raw key at runtime.
   */
  const resolves = (keys, key) =>
    keys.has(key) || (keys.has(`${key}_one`) && keys.has(`${key}_other`));
  const errors = [];
  for (const [key, sites] of collectTranslationCallSites()) {
    const missingIn = [];
    if (!resolves(enKeys, key)) missingIn.push("en");
    if (!resolves(zhKeys, key)) missingIn.push("zh-Hans");
    if (missingIn.length > 0) {
      errors.push(
        `unresolved translation key: ${key} (missing in ${missingIn.join(", ")}) at ${sites.join(", ")}`,
      );
    }
  }
  return errors;
}
/**
 * The one shape the literal call-site scan cannot see: `` t(`prefix-${x}`) ``.
 * Each exception declares the literal prefix, the complete set of suffixes the
 * option list can produce, and the file that builds the key. An undeclared
 * dynamic call site fails, so the pattern cannot be widened without extending
 * this list on purpose — the scan itself is never relaxed.
 */
export const DYNAMIC_KEY_EXCEPTIONS = [
  {
    prefix: "onboarding.backup.separator-",
    suffixes: ["spaces", "hyphens", "periods", "commas"],
    file: "features/onboarding/ui/EncryptedBackupCreator.tsx",
  },
];

const DYNAMIC_CALL_RE = /\bt\(\s*`([^`$]*)\$\{/g;

/** Every `` t(`prefix-${...}`) `` call site under `src`, by literal prefix. */
export function collectDynamicKeySites() {
  const sites = new Map();
  for (const file of sourceFiles(SRC_DIR)) {
    const text = withoutWholeLineComments(readFileSync(file, "utf8"));
    for (const match of text.matchAll(DYNAMIC_CALL_RE)) {
      const line = text.slice(0, match.index).split("\n").length;
      const at = `${path.relative(SRC_DIR, file).replace(/\\/g, "/")}:${line}`;
      const found = sites.get(match[1]);
      if (found) found.push(at);
      else sites.set(match[1], [at]);
    }
  }
  return sites;
}

/**
 * Check the declared dynamic-key exceptions against the source and the
 * catalogs: every form must exist in both, no form may be undeclared, and no
 * exception may outlive its call site.
 */
export function validateDynamicKeys(
  en,
  zhHans,
  exceptions = DYNAMIC_KEY_EXCEPTIONS,
  sites = collectDynamicKeySites(),
) {
  const enKeys = new Set(Object.keys(flattenCatalog(en)));
  const zhKeys = new Set(Object.keys(flattenCatalog(zhHans)));
  const resolves = (keys, key) =>
    keys.has(key) || (keys.has(`${key}_one`) && keys.has(`${key}_other`));
  const errors = [];

  const declared = new Set(exceptions.map((exception) => exception.prefix));
  for (const [prefix, prefixSites] of sites) {
    if (!declared.has(prefix)) {
      errors.push(
        `undeclared dynamic translation key: t(\`${prefix}...\`) at ${prefixSites.join(", ")} — declare it in DYNAMIC_KEY_EXCEPTIONS with its full suffix set`,
      );
    }
  }

  for (const exception of exceptions) {
    const prefixSites = sites.get(exception.prefix);
    if (!prefixSites) {
      errors.push(
        `stale dynamic translation exception: no t(\`${exception.prefix}...\`) call site remains for ${exception.file}`,
      );
      continue;
    }
    for (const suffix of exception.suffixes) {
      const key = `${exception.prefix}${suffix}`;
      const missingIn = [];
      if (!resolves(enKeys, key)) missingIn.push("en");
      if (!resolves(zhKeys, key)) missingIn.push("zh-Hans");
      if (missingIn.length > 0) {
        errors.push(
          `unresolved dynamic translation key: ${key} (missing in ${missingIn.join(", ")}) used at ${prefixSites.join(", ")}`,
        );
      }
    }
    const cataloged = new Set([
      ...[...enKeys, ...zhKeys].filter((key) =>
        key.startsWith(exception.prefix),
      ),
    ]);
    for (const key of cataloged) {
      const suffix = key.slice(exception.prefix.length);
      if (!exception.suffixes.includes(suffix.replace(/_(one|other)$/, ""))) {
        errors.push(
          `undeclared dynamic translation form: ${key} is not in the suffix set for ${exception.prefix}`,
        );
      }
    }
  }
  return errors;
}

/** CLI mode: `node catalogValidation.mjs` exits non-zero on problems. */
if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const { en, zhHans } = loadCatalogs();
  const problems = [
    ...validateCatalogParity(en, zhHans),
    ...validateTranslationCallSites(en, zhHans),
    ...validateDynamicKeys(en, zhHans),
  ];
  if (problems.length > 0) {
    for (const problem of problems) console.error(problem);
    process.exit(1);
  }
  console.log("catalog parity ok");
}
