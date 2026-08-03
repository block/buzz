#!/usr/bin/env node
/**
 * Model-capability manifest generator.
 *
 * Reads `scripts/model-capabilities.json` and emits:
 *   - `crates/buzz-agent/src/generated_model_capabilities.rs`
 *   - `desktop/src/features/agents/ui/modelCapabilities.ts`
 *
 * The generator performs three ordered resolution steps (resolver contract, plan v4):
 *   1. Provider-qualified raw exact lookup — key is (provider, raw_model_id), matched
 *      BEFORE any prefix stripping or normalization.
 *   2. Provider-scoped ordered family rules — match kinds: exact | prefix | gpt5-token |
 *      gpt5-base | segment | segment-prefix, ordered by match_priority (desc).
 *   3. Per-axis provider fallback — blank vs concrete-unknown states.
 *
 * Completeness validator: every emitted CapabilityResult must have every axis populated.
 * A generation-time failure on any unresolvable axis is a hard error.
 *
 * Usage:
 *   node scripts/generate-model-capabilities.mjs [--check]
 *
 * --check: verify that already-generated files match what the generator would produce
 *          (used by CI regenerate-then-diff job). Exits 1 if any file differs.
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, "..");
const CHECK_MODE = process.argv.includes("--check");
const CHECK_GOOSE_MODE = process.argv.includes("--check-goose");

// Run content through rustfmt (stdin → stdout) if available; fall back to raw.
// Uses hermit-pinned rustfmt from bin/ when present (same binary as pre-commit hook).
function rustfmt(content) {
  const hermitBin = join(repoRoot, "bin", "rustfmt");
  const hermitExists = existsSync(hermitBin);
  const rustfmtBin = hermitExists ? hermitBin : "rustfmt";
  const result = spawnSync(rustfmtBin, ["--edition", "2021", "--emit", "stdout"], {
    input: content,
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });
  if (result.status === 0 && result.stdout) return result.stdout;
  if (hermitExists && result.status !== 0) {
    // Hermit-pinned binary is present but failed — fail closed so CI catches drift.
    const stderr = result.stderr ?? "";
    throw new Error(`rustfmt (hermit-pinned) exited ${result.status}: ${stderr.trim()}`);
  }
  // rustfmt not available (PATH fallback also absent) — warn and return raw.
  // CI's regen-diff gate will catch any drift.
  process.stderr.write("warning: rustfmt not available; raw Rust output may not be formatter-stable\n");
  return content;
}

// Support --manifest-path and --output-dir flags for testing
const manifestPathOverride = (() => {
  const idx = process.argv.indexOf("--manifest-path");
  return idx !== -1 ? process.argv[idx + 1] : null;
})();
const outputDirOverride = (() => {
  const idx = process.argv.indexOf("--output-dir");
  return idx !== -1 ? process.argv[idx + 1] : null;
})();

// ---------------------------------------------------------------------------
// Load manifest
// ---------------------------------------------------------------------------

const manifestPath = manifestPathOverride ?? join(repoRoot, "scripts", "model-capabilities.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

// Validate registry_labels — must be an array of {id, label} objects with unique IDs.
// JSON.parse() silently overwrites duplicate object keys, so an array is required for
// structural duplicate detection.
const registryLabelsArr = manifest.registry_labels ?? [];
if (!Array.isArray(registryLabelsArr)) {
  throw new Error("registry_labels: must be an array of {id, label} objects");
}
for (const entry of registryLabelsArr) {
  if (!entry.id || typeof entry.id !== "string" || entry.id.trim() === "") {
    throw new Error(`registry_labels: entry missing nonempty string "id": ${JSON.stringify(entry)}`);
  }
  if (!entry.label || typeof entry.label !== "string" || entry.label.trim() === "") {
    throw new Error(`registry_labels: entry id="${entry.id}" missing nonempty string "label"`);
  }
  // Safe for code interpolation: reject double-quote, backslash, and control chars.
  // (These characters would break emitted Rust/TS string literals.)
  // Checked via requireSafeString() below (shared validator defined after knownModels block).
  const unsafeId = entry.id.includes('"') || entry.id.includes("\\") ||
    Array.from(entry.id).some((c) => c.charCodeAt(0) < 32);
  if (unsafeId) {
    throw new Error(`registry_labels: entry id="${entry.id}" contains unsafe characters`);
  }
  const unsafeLabel = entry.label.includes('"') || entry.label.includes("\\") ||
    Array.from(entry.label).some((c) => c.charCodeAt(0) < 32);
  if (unsafeLabel) {
    throw new Error(`registry_labels: entry id="${entry.id}" label contains unsafe characters`);
  }
}
const registryLabelIds = registryLabelsArr.map((e) => e.id);
const registryLabelSet = new Set(registryLabelIds);
if (registryLabelSet.size !== registryLabelIds.length) {
  const dups = registryLabelIds.filter((id, i) => registryLabelIds.indexOf(id) !== i);
  throw new Error(`registry_labels: duplicate endpoint IDs detected: ${dups.join(", ")}`);
}

// Validate databricks_v2_known_models uniqueness
const knownModels = manifest.databricks_v2_known_models ?? [];
const knownModelsSet = new Set(knownModels);
if (knownModelsSet.size !== knownModels.length) {
  throw new Error(`databricks_v2_known_models: duplicate IDs detected: ${
    knownModels.filter((id, i) => knownModels.indexOf(id) !== i).join(", ")
  }`);
}

// ---------------------------------------------------------------------------
// Shared string-safety validator
// ---------------------------------------------------------------------------
//
// All manifest strings that end up inside generated Rust or TypeScript string literals
// must not contain double-quote, backslash, or control characters — any of these would
// break the emitted source or allow injection. One shared check prevents the same class
// of defect from appearing at scattered emission sites.
//
// Usage: requireSafeString(value, "context.path") — throws on violation.
// ---------------------------------------------------------------------------

function requireSafeString(value, context) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${context}: must be a nonempty string, got ${JSON.stringify(value)}`);
  }
  const hasUnsafe = value.includes('"') || value.includes("\\") ||
    Array.from(value).some((c) => c.charCodeAt(0) < 32);
  if (hasUnsafe) {
    throw new Error(`${context}: contains unsafe characters (double-quote, backslash, or control chars): ${JSON.stringify(value)}`);
  }
}

// Validate all manifest strings that end up in generated Rust/TS string literals.
// family_tokens → Rust FAMILY_TOKENS array and TS FAMILY_TOKENS constant
for (const tok of manifest.family_tokens ?? []) {
  requireSafeString(tok, "family_tokens[]");
}

// databricks_v2_known_models IDs → Rust/TS DATABRICKS_V2_KNOWN_MODELS array
for (const id of knownModels) {
  requireSafeString(id, "databricks_v2_known_models[]");
}

// exact_records → Rust lookup_exact() match arms and TS EXACT_RECORDS map keys
for (const rec of manifest.exact_records ?? []) {
  requireSafeString(rec.provider ?? "", `exact_records[${rec.raw_model_id}].provider`);
  requireSafeString(rec.raw_model_id ?? "", `exact_records[${rec.raw_model_id}].raw_model_id`);
}

// family_rules → Rust/TS lookup_by_family_rules() match-expression strings
for (const rule of manifest.family_rules ?? []) {
  requireSafeString(rule.id ?? "", `family_rules[${rule.id}].id`);
  requireSafeString(rule.match_value ?? "", `family_rules[${rule.id}].match_value`);
  for (const alias of rule.match_aliases ?? []) {
    requireSafeString(alias, `family_rules[${rule.id}].match_aliases[]`);
  }
  for (const provider of rule.providers ?? []) {
    requireSafeString(provider, `family_rules[${rule.id}].providers[]`);
  }
  // registry_label flows into Rust Some("...") and TS "..." string literals
  if (rule.registry_label != null) {
    requireSafeString(rule.registry_label, `family_rules[${rule.id}].registry_label`);
  }
}

// exact_records: registry_label values flow into Rust rustString() and TS emitter
for (const rec of manifest.exact_records ?? []) {
  if (rec.registry_label != null) {
    requireSafeString(rec.registry_label, `exact_records[${rec.raw_model_id}].registry_label`);
  }
}

// provider_fallbacks: object keys are interpolated as Rust match arms and TS object keys
for (const providerKey of Object.keys(manifest.provider_fallbacks ?? {})) {
  if (providerKey !== "_default") {
    requireSafeString(providerKey, `provider_fallbacks key`);
  }
}

// ---------------------------------------------------------------------------
// --check-goose: optional drift check against pinned goose upstream
// ---------------------------------------------------------------------------
//
// Reads the goose source file at the pinned revision and verifies that
// manifest.databricks_v2_known_models exactly matches the IDs declared there.
//
// Usage (non-CI, opt-in):
//   node scripts/generate-model-capabilities.mjs --check-goose
//
// Pin metadata lives in manifest._sources.goose_known_models:
//   "goose revision <SHA> (<path>:<lines>) — <ids>"
//
// The check fetches the raw file from github.com/block/goose via the GitHub
// contents API and parses DATABRICKS_V2_KNOWN_MODELS from it.
// ---------------------------------------------------------------------------

if (CHECK_GOOSE_MODE) {
  // Parse pin metadata from _sources
  const pin = manifest._sources?.goose_known_models;
  if (!pin) {
    console.error("--check-goose: manifest._sources.goose_known_models is missing.");
    process.exit(1);
  }
  const revMatch = pin.match(/revision\s+([0-9a-f]{7,40})/i);
  const pathMatch = pin.match(/\(([^)]+\.rs):/);
  if (!revMatch || !pathMatch) {
    console.error(`--check-goose: could not parse revision/path from pin metadata: ${pin}`);
    process.exit(1);
  }
  const pinnedRev = revMatch[1];
  const goosePath = pathMatch[1]; // e.g. "crates/goose-providers/src/databricks_v2.rs"

  // Fetch file content from GitHub API (no token required for public repos)
  const url = `https://raw.githubusercontent.com/block/goose/${pinnedRev}/${goosePath}`;
  let fileContent;
  try {
    const result = spawnSync("curl", ["-fsS", "--max-time", "15", url], {
      encoding: "utf8",
      maxBuffer: 2 * 1024 * 1024,
    });
    if (result.status !== 0) {
      console.error(`--check-goose: curl failed (exit ${result.status}): ${result.stderr?.trim()}`);
      console.error(`  URL: ${url}`);
      process.exit(1);
    }
    fileContent = result.stdout;
  } catch (e) {
    console.error(`--check-goose: fetch failed: ${e.message}`);
    process.exit(1);
  }

  // Extract DATABRICKS_V2_KNOWN_MODELS from Rust source: parse &[&str] literal
  // Pattern: pub const DATABRICKS_V2_KNOWN_MODELS: &[&str] = &[ "id1", "id2", ... ];
  const constMatch = fileContent.match(
    /DATABRICKS_V2_KNOWN_MODELS\s*:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/,
  );
  if (!constMatch) {
    console.error(`--check-goose: could not find DATABRICKS_V2_KNOWN_MODELS in goose source at ${pinnedRev}:${goosePath}`);
    process.exit(1);
  }
  const gooseIds = [...constMatch[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]).sort();
  const manifestIds = [...(manifest.databricks_v2_known_models ?? [])].sort();

  const onlyInGoose = gooseIds.filter((id) => !manifestIds.includes(id));
  const onlyInManifest = manifestIds.filter((id) => !gooseIds.includes(id));

  if (onlyInGoose.length === 0 && onlyInManifest.length === 0) {
    console.log(`--check-goose: OK — manifest matches goose@${pinnedRev} (${gooseIds.length} IDs)`);
  } else {
    if (onlyInGoose.length > 0) {
      console.error(`--check-goose: DRIFT — IDs in goose@${pinnedRev} not in manifest: ${onlyInGoose.join(", ")}`);
    }
    if (onlyInManifest.length > 0) {
      console.error(`--check-goose: DRIFT — IDs in manifest not in goose@${pinnedRev}: ${onlyInManifest.join(", ")}`);
    }
    console.error(`Pin: ${pin}`);
    process.exit(1);
  }
  process.exit(0);
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

const VALID_EFFORTS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
const VALID_THINKING_MODES = ["manual-budget", "adaptive", "omit-fields", "none", "not-applicable"];
const VALID_DBV2_ROUTES = [
  "openai-responses",
  "anthropic-messages",
  "mlflow-chat",
  "route-unknown",
  "not-applicable",
];
const VALID_NORM_POLICIES = ["none", "openai-standard", "openai-clamp-max-to-xhigh"];
const VALID_MATCH_KINDS = [
  "exact",
  "prefix",
  "gpt5-token",
  "gpt5-base",
  "segment",
  "segment-prefix",
];

function assertEnum(value, valid, label) {
  if (!valid.includes(value)) {
    throw new Error(`${label}: invalid value "${value}" (must be one of: ${valid.join(", ")})`);
  }
}

function assertNonEmpty(arr, label) {
  if (!Array.isArray(arr) || arr.length === 0) {
    throw new Error(`${label}: must be a non-empty array`);
  }
}

function validateFallbackRecord(rec, label) {
  assertEnum(rec.databricks_v2_wire_route, VALID_DBV2_ROUTES, `${label}.databricks_v2_wire_route`);
  assertEnum(rec.thinking_mode, VALID_THINKING_MODES, `${label}.thinking_mode`);
  assertNonEmpty(rec.supported_efforts, `${label}.supported_efforts`);
  for (const e of rec.supported_efforts) {
    assertEnum(e, VALID_EFFORTS, `${label}.supported_efforts[]`);
  }
  if (rec.default_effort !== null) {
    assertEnum(rec.default_effort, VALID_EFFORTS, `${label}.default_effort`);
    if (!rec.supported_efforts.includes(rec.default_effort)) {
      throw new Error(
        `${label}: default_effort "${rec.default_effort}" not in supported_efforts [${rec.supported_efforts.join(", ")}]`,
      );
    }
  }
  assertEnum(rec.normalization_policy, VALID_NORM_POLICIES, `${label}.normalization_policy`);
}

// ---------------------------------------------------------------------------
// Validate manifest structure
// ---------------------------------------------------------------------------

// Validate family_rules
const seenRuleIds = new Set();
for (const rule of manifest.family_rules) {
  if (!rule.id) throw new Error("family_rule missing id");
  if (seenRuleIds.has(rule.id)) throw new Error(`duplicate family_rule id: ${rule.id}`);
  seenRuleIds.add(rule.id);
  assertEnum(rule.match_kind, VALID_MATCH_KINDS, `rule ${rule.id} match_kind`);
  assertEnum(rule.thinking_mode, VALID_THINKING_MODES, `rule ${rule.id} thinking_mode`);
  assertNonEmpty(rule.supported_efforts, `rule ${rule.id} supported_efforts`);
  for (const e of rule.supported_efforts) {
    assertEnum(e, VALID_EFFORTS, `rule ${rule.id} supported_efforts[]`);
  }
  if (rule.default_effort !== null) {
    assertEnum(rule.default_effort, VALID_EFFORTS, `rule ${rule.id} default_effort`);
    if (!rule.supported_efforts.includes(rule.default_effort)) {
      throw new Error(
        `rule ${rule.id}: default_effort "${rule.default_effort}" not in supported_efforts`,
      );
    }
  }
  assertEnum(
    rule.databricks_v2_wire_route,
    VALID_DBV2_ROUTES,
    `rule ${rule.id} databricks_v2_wire_route`,
  );
  assertEnum(
    rule.normalization_policy,
    VALID_NORM_POLICIES,
    `rule ${rule.id} normalization_policy`,
  );
}

// Validate provider_fallbacks
for (const [provider, fb] of Object.entries(manifest.provider_fallbacks)) {
  for (const state of ["blank", "concrete_unknown"]) {
    if (!fb[state])
      throw new Error(`provider_fallbacks.${provider} missing "${state}" record`);
    validateFallbackRecord(fb[state], `provider_fallbacks.${provider}.${state}`);
  }
}

// Validate exact_records — check for duplicate (provider, raw_model_id) keys
const seenExactKeys = new Set();
for (const rec of manifest.exact_records ?? []) {
  if (!rec.provider || !rec.raw_model_id)
    throw new Error("exact_record missing provider or raw_model_id");
  const key = `${rec.provider}::${rec.raw_model_id}`;
  if (seenExactKeys.has(key)) throw new Error(`duplicate exact_record key: ${key}`);
  seenExactKeys.add(key);
}

// ---------------------------------------------------------------------------
// Resolution engine (mirrors plan resolver contract)
// ---------------------------------------------------------------------------

/**
 * Strip catalog prefix to get the normalized alias for family-rule matching.
 * Finds the first occurrence of a known family token and returns from there.
 * e.g. "goose-claude-fable-5" → "claude-fable-5"
 *      "databricks-gpt-5.5"   → "gpt-5.5"
 *      "claude-opus-4-7"      → "claude-opus-4-7"  (no prefix)
 */
function stripCatalogPrefix(model) {
  const lower = model.toLowerCase();
  let firstIdx = Infinity;
  for (const tok of manifest.family_tokens) {
    const idx = lower.indexOf(tok);
    if (idx !== -1 && idx < firstIdx) firstIdx = idx;
  }
  return firstIdx === Infinity ? model : model.slice(firstIdx);
}

/**
 * gpt5-token match: model contains token at a word boundary (end-of-string or "-").
 * Does NOT match if followed by a digit or letter.
 */
function gpt5TokenMatches(model, token) {
  const lower = model.toLowerCase();
  let start = 0;
  while (true) {
    const idx = lower.indexOf(token.toLowerCase(), start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    const afterChar = afterIdx < lower.length ? lower[afterIdx] : "";
    if (afterChar === "" || afterChar === "-") return true;
    start = afterIdx;
  }
}

/**
 * gpt5-base match: like gpt5-token but also rejects short -<1-3 digit> suffixes.
 */
function gpt5BaseMatches(model, token) {
  const lower = model.toLowerCase();
  let start = 0;
  while (true) {
    const idx = lower.indexOf(token.toLowerCase(), start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    const suffix = lower.slice(afterIdx);
    if (suffix === "") return true;
    if (!suffix.startsWith("-")) {
      start = afterIdx;
      continue;
    }
    const dashRest = suffix.slice(1);
    if (/^\d{1,3}(?:[^a-z\d]|$)/i.test(dashRest)) {
      start = afterIdx;
      continue;
    }
    return true;
  }
}

/**
 * Test if a family rule matches the given (normalized) model string for a provider.
 */
function ruleMatchesModel(rule, normalizedModel, provider) {
  if (!rule.providers.includes(provider)) return false;
  const lower = normalizedModel.toLowerCase();
  const allTokens = [rule.match_value, ...(rule.match_aliases ?? [])];
  switch (rule.match_kind) {
    case "exact":
      return allTokens.some((t) => lower === t.toLowerCase());
    case "prefix":
      return allTokens.some((t) => lower.startsWith(t.toLowerCase()));
    case "gpt5-token":
      return allTokens.some((t) => gpt5TokenMatches(lower, t));
    case "gpt5-base":
      return allTokens.some((t) => gpt5BaseMatches(lower, t));
    case "segment": {
      const segs = lower.split(/[^a-z0-9]+/);
      return allTokens.some((t) => segs.includes(t.toLowerCase()));
    }
    case "segment-prefix": {
      const segs = lower.split(/[^a-z0-9]+/);
      return allTokens.some((t) => segs.some((s) => s.startsWith(t.toLowerCase())));
    }
    default:
      throw new Error(`unknown match_kind: ${rule.match_kind}`);
  }
}

/**
 * Resolve (provider, rawModelId) → CapabilityResult.
 * Three steps: raw exact lookup → family rules → provider fallback.
 */
function resolve(provider, rawModelId) {
  const isBlank = !rawModelId || rawModelId.trim() === "";

  // Step 1: provider-qualified raw exact lookup (BEFORE any prefix stripping)
  const exactRecord = (manifest.exact_records ?? []).find(
    (r) => r.provider === provider && r.raw_model_id === rawModelId,
  );

  if (exactRecord) {
    // Materialize the full record by evaluating family rules for any axis not overridden.
    const normalizedAlias = stripCatalogPrefix(rawModelId);
    const familyResult = resolveFamilyRules(provider, normalizedAlias, rawModelId);
    const base = familyResult ?? getProviderFallback(provider, isBlank);
    // Determine which axes came from family rule vs exact record for per-axis provenance.
    const effortsFromExact = exactRecord.supported_efforts_override !== undefined;
    const routeFromExact = exactRecord.databricks_v2_wire_route !== undefined;
    const modeFromExact = exactRecord.thinking_mode !== undefined;
    const normFromExact = exactRecord.normalization_policy !== undefined;
    const effortDefaultFromExact = exactRecord.default_effort !== undefined;
    const labelFromExact = exactRecord.registry_label !== undefined;
    const familyProv = familyResult
      ? { rule_id: familyResult._provenance.rule_id, rule_priority: familyResult._provenance.rule_priority }
      : null;
    return {
      registry_label: exactRecord.registry_label ?? null,
      thinking_mode: exactRecord.thinking_mode ?? base.thinking_mode,
      supported_efforts: exactRecord.supported_efforts_override ?? base.supported_efforts,
      default_effort: exactRecord.default_effort !== undefined ? exactRecord.default_effort : base.default_effort,
      databricks_v2_wire_route: exactRecord.databricks_v2_wire_route ?? base.databricks_v2_wire_route,
      normalization_policy: exactRecord.normalization_policy ?? base.normalization_policy,
      _provenance: {
        source: "exact",
        exact_key: `${provider}::${rawModelId}`,
        registry_label: labelFromExact ? "exact_record" : (familyProv ? `family:${familyProv.rule_id}@${familyProv.rule_priority}` : "absent"),
        supported_efforts: effortsFromExact ? "exact_record" : (familyProv ? `family:${familyProv.rule_id}@${familyProv.rule_priority}` : "fallback"),
        databricks_v2_wire_route: routeFromExact ? "exact_record" : (familyProv ? `family:${familyProv.rule_id}@${familyProv.rule_priority}` : "fallback"),
        thinking_mode: modeFromExact ? "exact_record" : (familyProv ? `family:${familyProv.rule_id}@${familyProv.rule_priority}` : "fallback"),
        normalization_policy: normFromExact ? "exact_record" : (familyProv ? `family:${familyProv.rule_id}@${familyProv.rule_priority}` : "fallback"),
        default_effort: effortDefaultFromExact ? "exact_record" : (familyProv ? `family:${familyProv.rule_id}@${familyProv.rule_priority}` : "fallback"),
      },
    };
  }

  // Step 2: provider-scoped family rules on normalized alias
  const normalizedAlias = stripCatalogPrefix(rawModelId ?? "");
  const familyResult = resolveFamilyRules(provider, normalizedAlias, rawModelId);
  if (familyResult) return familyResult;

  // Step 3: provider fallback
  const fallback = getProviderFallback(provider, isBlank);
  return { ...fallback, registry_label: null };
}

function resolveFamilyRules(provider, normalizedAlias, rawModelId) {
  if (!normalizedAlias) return null;

  // Sort rules by match_priority descending (higher priority wins)
  const sorted = [...manifest.family_rules].sort((a, b) => b.match_priority - a.match_priority);

  for (const rule of sorted) {
    if (ruleMatchesModel(rule, normalizedAlias, provider)) {
      // databricks_v2_wire_route is only meaningful for databricks_v2; all other providers get not-applicable
      const wireRoute = provider === "databricks_v2"
        ? rule.databricks_v2_wire_route
        : "not-applicable";
      return {
        registry_label: rule.registry_label ?? null,
        thinking_mode: rule.thinking_mode,
        supported_efforts: rule.supported_efforts,
        default_effort: rule.default_effort,
        databricks_v2_wire_route: wireRoute,
        normalization_policy: rule.normalization_policy,
        _provenance: {
          source: "family",
          rule_id: rule.id,
          rule_priority: rule.match_priority,
          normalized_alias: normalizedAlias,
          raw_model_id: rawModelId,
        },
      };
    }
  }
  return null;
}

function getProviderFallback(provider, isBlank) {
  const fb =
    manifest.provider_fallbacks[provider] ?? manifest.provider_fallbacks["_default"];
  const state = isBlank ? "blank" : "concrete_unknown";
  const rec = fb[state];
  return {
    ...rec,
    _provenance: { source: "fallback", provider, state },
  };
}

// ---------------------------------------------------------------------------
// Rust code generation
// ---------------------------------------------------------------------------

function rustString(s) {
  if (s === null || s === undefined) return "None";
  return `Some("${s}")`;
}

function rustEffortList(efforts) {
  const mapped = efforts.map((e) => `ThinkingEffort::${capitalize(e)}`);
  return `&[${mapped.join(", ")}]`;
}

function rustEffortOption(e) {
  if (e === null || e === undefined) return "None";
  return `Some(ThinkingEffort::${capitalize(e)})`;
}

function capitalize(s) {
  if (s === "xhigh") return "XHigh";
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function rustDbv2Route(route) {
  const map = {
    "openai-responses": "DatabricksV2Route::OpenAiResponses",
    "anthropic-messages": "DatabricksV2Route::AnthropicMessages",
    "mlflow-chat": "DatabricksV2Route::MlflowChatCompletions",
    "route-unknown": "DatabricksV2Route::RouteUnknown",
    "not-applicable": "DatabricksV2Route::NotApplicable",
  };
  if (!map[route]) throw new Error(`unknown dbv2 route: ${route}`);
  return map[route];
}

function rustNormPolicy(policy) {
  const map = {
    none: "NormalizationPolicy::None",
    "openai-standard": "NormalizationPolicy::OpenAiStandard",
    "openai-clamp-max-to-xhigh": "NormalizationPolicy::OpenAiClampMaxToXHigh",
  };
  if (!map[policy]) throw new Error(`unknown norm policy: ${policy}`);
  return map[policy];
}

function rustThinkingMode(mode) {
  const map = {
    "manual-budget": "ThinkingMode::ManualBudget",
    adaptive: "ThinkingMode::Adaptive",
    "omit-fields": "ThinkingMode::OmitFields",
    none: "ThinkingMode::None",
    "not-applicable": "ThinkingMode::NotApplicable",
  };
  if (!map[mode]) throw new Error(`unknown thinking mode: ${mode}`);
  return map[mode];
}

function emitRustCapabilityResult(r, indent = "    ") {
  const i = indent;
  const lines = [
    `${i}CapabilityResult {`,
    `${i}    registry_label: ${rustString(r.registry_label)},`,
    `${i}    thinking_mode: ${rustThinkingMode(r.thinking_mode)},`,
    `${i}    supported_efforts: Cow::Borrowed(${rustEffortList(r.supported_efforts)}),`,
    `${i}    default_effort: ${rustEffortOption(r.default_effort)},`,
    `${i}    databricks_v2_wire_route: ${rustDbv2Route(r.databricks_v2_wire_route)},`,
    `${i}    normalization_policy: ${rustNormPolicy(r.normalization_policy)},`,
    `${i}}`,
  ];
  return lines.join("\n");
}

// Build the exact_records map entries for Rust
const exactMapEntries = [];
for (const rec of manifest.exact_records ?? []) {
  const result = resolve(rec.provider, rec.raw_model_id);
  // Strip provenance from emitted result
  const clean = { ...result };
  delete clean._provenance;
  const provNote = result._provenance
    ? (() => {
        const p = result._provenance;
        if (p.source !== "exact") return `// source: ${p.source}`;
        return [
          `// provenance: exact(${p.exact_key})`,
          `//   registry_label: ${p.registry_label}`,
          `//   supported_efforts: ${p.supported_efforts}`,
          `//   databricks_v2_wire_route: ${p.databricks_v2_wire_route}`,
          `//   thinking_mode: ${p.thinking_mode}`,
          `//   normalization_policy: ${p.normalization_policy}`,
          `//   default_effort: ${p.default_effort}`,
        ].join("\n            ");
      })()
    : "";
  exactMapEntries.push({ rec, clean, provNote });
}

// Build provider fallback static arrays
function emitRustFallbackFn(provider, state, rec) {
  const clean = { ...rec };
  delete clean._provenance;
  const stateId = state === "blank" ? "Blank" : "ConcreteUnknown";
  return emitRustCapabilityResult(clean, "        ");
}

// Collect all providers for fallback fns
const providerFallbackKeys = Object.keys(manifest.provider_fallbacks).filter(
  (k) => k !== "_default",
);

const rustContent = `// @generated — do not edit by hand.
// Regenerate with: node scripts/generate-model-capabilities.mjs
// Source: scripts/model-capabilities.json

//! Generated model-capability lookup tables.
//!
//! Resolution is a total function \`resolve(provider, raw_model_id) → CapabilityResult\`.
//! Three ordered steps (plan v4 resolver contract):
//! 1. Provider-qualified raw exact lookup (before any prefix stripping).
//! 2. Provider-scoped ordered family rules on the normalized alias.
//! 3. Per-axis provider fallback (blank vs concrete-unknown).
//!
//! The manifest (scripts/model-capabilities.json) is the single source of truth.
//! This file is regenerated by scripts/generate-model-capabilities.mjs.
//! CI verifies that generated files match the manifest (regenerate-then-diff).
//!
//! Boundaries this file does NOT own (plan v4 §Boundaries):
//! - Transport/endpoint selection for pure OpenAI, legacy Databricks, OpenRouter:
//!   OpenAiApi / openai_request() remain authoritative.
//! - Final display labels: resolveModelLabel() three-tier precedence is authoritative.
//!   \`registry_label\` here feeds only the static registry tier.
//! - llm.rs replacement scope: ONLY \`databricks_v2_route_for_model\`.

use crate::config::ThinkingEffort;
use std::borrow::Cow;

/// Which Databricks v2 gateway wire path to use for a model.
/// Scoped to DBv2 only — other providers use \`NotApplicable\`.
/// Transport for pure OpenAI, legacy Databricks, and OpenRouter is selected
/// by OpenAiApi / openai_request() at runtime, not by this manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabricksV2Route {
    /// /ai-gateway/openai/v1/responses
    OpenAiResponses,
    /// /ai-gateway/anthropic/v1/messages
    AnthropicMessages,
    /// /ai-gateway/mlflow/v1/chat/completions
    MlflowChatCompletions,
    /// DBv2 blank model — route not yet determinable.
    RouteUnknown,
    /// Not a DBv2 provider — transport is selected by OpenAiApi/openai_request().
    NotApplicable,
}

/// Anthropic thinking API shape for this model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    /// thinking:{type:"enabled", budget_tokens} — claude-3*, claude-opus-4-5
    ManualBudget,
    /// thinking:{type:"adaptive"} + output_config:{effort} — opus-4-6+, sonnet-4-6+, etc.
    Adaptive,
    /// Unknown Anthropic model — omit thinking fields rather than guess request shape.
    OmitFields,
    /// Non-Anthropic-routed model — thinking fields are not applicable.
    None,
    /// Provider does not use Anthropic thinking API at all.
    NotApplicable,
}

/// How to normalize effort values before sending to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationPolicy {
    /// Pass effort through unchanged.
    None,
    /// Apply per-family effort table; none↔minimal peer fallback + upward tie preference.
    OpenAiStandard,
    /// Unknown OpenAI model: clamp max → xhigh, pass all others unchanged.
    OpenAiClampMaxToXHigh,
}

/// Complete resolved capability record for a (provider, raw_model_id) pair.
/// Every axis is populated — runtime consumers do not compose fields.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityResult {
    /// Optional static display label. Feeds resolveModelLabel()'s registry tier only.
    /// The dynamic three-tier precedence (discovered_name > registry_label > raw_id)
    /// lives in formatAgentModelLabel / resolveModelLabel — NOT in this struct.
    pub registry_label: Option<&'static str>,
    /// Anthropic thinking API shape for this model.
    pub thinking_mode: ThinkingMode,
    /// Valid effort values for the model's effort dropdown (UI).
    pub supported_efforts: Cow<'static, [ThinkingEffort]>,
    /// Semantic default, or None when "Inherit" is the natural default (manual-budget Anthropic).
    pub default_effort: Option<ThinkingEffort>,
    /// DBv2 wire route; NotApplicable for non-DBv2 providers.
    pub databricks_v2_wire_route: DatabricksV2Route,
    /// Effort normalization policy before sending to provider.
    pub normalization_policy: NormalizationPolicy,
}

// ---------------------------------------------------------------------------
// Exact records — provider-qualified (provider, raw_model_id), pre-prefix-stripping
// ---------------------------------------------------------------------------

/// Returns the exact capability record for a provider-qualified raw model ID,
/// if one exists in the manifest. This is checked BEFORE any prefix stripping.
pub fn lookup_exact(provider: &str, raw_model_id: &str) -> Option<CapabilityResult> {
    match (provider, raw_model_id) {
${exactMapEntries
  .map(({ rec, clean, provNote }) => {
    return `        ("${rec.provider}", "${rec.raw_model_id}") => {
            ${provNote}
            Some(
${emitRustCapabilityResult(clean, "                ")}
            )
        }`;
  })
  .join("\n")}
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Family rule resolution — normalized alias after prefix stripping
// ---------------------------------------------------------------------------

/// Strip any catalog-naming prefix to get the normalized model alias for family matching.
/// Finds the first occurrence of a known family token (claude-, gpt-) and returns from there.
///
/// Examples:
///   "goose-claude-fable-5"      → "claude-fable-5"
///   "databricks-gpt-5.5"        → "gpt-5.5"
///   "team-x-claude-opus-4-7"    → "claude-opus-4-7"
///   "claude-opus-4-7"           → "claude-opus-4-7"  (no prefix)
///   "llama-3"                   → "llama-3"           (no family token)
pub fn strip_catalog_prefix(model: &str) -> &str {
    const FAMILY_TOKENS: &[&str] = &[${manifest.family_tokens.map((t) => `"${t}"`).join(", ")}];
    let lower = model.to_ascii_lowercase();
    let first_idx = FAMILY_TOKENS
        .iter()
        .filter_map(|tok| lower.find(tok))
        .min();
    match first_idx {
        Some(idx) => &model[idx..],
        None => model,
    }
}

${emitRustFamilyResolverFn()}

// ---------------------------------------------------------------------------
// Provider fallbacks — blank vs concrete-unknown, per provider
// ---------------------------------------------------------------------------

/// Returns the fallback capability record for the given provider and model state.
/// \`is_blank\` is true when the model string is empty/whitespace; false when it is
/// a nonblank but unmatched concrete ID.
pub fn provider_fallback(provider: &str, is_blank: bool) -> CapabilityResult {
    match (provider, is_blank) {
${providerFallbackKeys
  .map((provider) => {
    const fb = manifest.provider_fallbacks[provider];
    const blankClean = { ...fb.blank };
    delete blankClean._provenance;
    const concClean = { ...fb.concrete_unknown };
    delete concClean._provenance;
    return `        ("${provider}", true) => {
${emitRustCapabilityResult({ ...blankClean, registry_label: null }, "            ")}
        }
        ("${provider}", false) => {
${emitRustCapabilityResult({ ...concClean, registry_label: null }, "            ")}
        }`;
  })
  .join("\n")}
        // Default fallback for unknown/empty providers
        (_, true) => {
${emitRustCapabilityResult(
  {
    ...manifest.provider_fallbacks["_default"].blank,
    registry_label: null,
  },
  "            ",
)}
        }
        (_, false) => {
${emitRustCapabilityResult(
  {
    ...manifest.provider_fallbacks["_default"].concrete_unknown,
    registry_label: null,
  },
  "            ",
)}
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level resolve — total function, always returns a complete result
// ---------------------------------------------------------------------------

/// Resolve (provider, raw_model_id) → CapabilityResult.
///
/// This is the single entry point. Result is complete — every axis is populated.
/// Consumers never compose fields from multiple tiers at runtime.
///
/// Resolution order (plan v4 resolver contract):
///   1. provider-qualified raw exact lookup (before any prefix stripping)
///   2. provider-scoped family rules on normalized alias
///   3. per-axis provider fallback (blank vs concrete-unknown)
pub fn resolve_model_capabilities(provider: &str, raw_model_id: &str) -> CapabilityResult {
    // Step 1: raw exact lookup
    if let Some(exact) = lookup_exact(provider, raw_model_id) {
        return exact;
    }

    // Step 2: family rules on normalized alias
    let normalized = strip_catalog_prefix(raw_model_id);
    if let Some(family) = lookup_by_family_rules(provider, normalized) {
        return family;
    }

    // Step 3: provider fallback
    let is_blank = raw_model_id.trim().is_empty();
    let mut fallback = provider_fallback(provider, is_blank);
    fallback.registry_label = None;
    fallback
}

// ---------------------------------------------------------------------------
// Generated constants (fold-in from #3603 branch)
// ---------------------------------------------------------------------------

/// Valid thinking-effort values accepted by buzz-agent.
/// Mirrors parse_thinking_effort in config.rs.
pub const THINKING_EFFORT_VALUES: &[&str] = &[${VALID_EFFORTS.map((e) => `"${e}"`).join(", ")}];

/// Databricks v2 known model IDs. Mirrors goose DATABRICKS_V2_KNOWN_MODELS.
/// Single source of truth inside buzz; generated from manifest databricks_v2_known_models section.
pub const DATABRICKS_V2_KNOWN_MODELS: &[&str] = &[
${manifest.databricks_v2_known_models
  .map((id) => `    "${id}",`)
  .join("\n")}
];

/// Databricks endpoint-ID to display-name registry. Generated from manifest registry_labels section.
/// Feeds the static registry tier of resolveModelLabel(). Final display label is determined
/// by the three-tier precedence in resolveModelLabel() (discovered_name > registry_label > raw_id).
pub const DATABRICKS_MODEL_NAMES: &[(&str, &str)] = &[
${registryLabelsArr
  .map(({ id, label }) => `    ("${id}", "${label}"),`)
  .join("\n")}
];
`;

function emitRustFamilyResolverFn() {
  // Generate match arms for each family rule, sorted by priority desc
  const sorted = [...manifest.family_rules].sort((a, b) => b.match_priority - a.match_priority);

  // Group by provider for the generated fn
  const allProviders = [
    ...new Set(sorted.flatMap((r) => r.providers)),
  ];

  // We emit a single fn that takes (provider: &str, normalized: &str)
  // and returns Option<CapabilityResult>. We use if-else chains.

  const arms = [];
  for (const rule of sorted) {
    for (const provider of rule.providers) {
      const clean = { ...rule };
      delete clean._provenance;
      // databricks_v2_wire_route is only meaningful for databricks_v2; all other providers get not-applicable
      const wireRoute = provider === "databricks_v2"
        ? rule.databricks_v2_wire_route
        : "not-applicable";
      const matchExpr = buildRustMatchExpr(rule, provider);
      arms.push(
        `    // rule: ${rule.id}, provider: ${provider}, priority: ${rule.match_priority}\n    if provider == "${provider}" && (${matchExpr}) {\n        return Some(\n${emitRustCapabilityResult(
          {
            registry_label: rule.registry_label ?? null,
            thinking_mode: rule.thinking_mode,
            supported_efforts: rule.supported_efforts,
            default_effort: rule.default_effort,
            databricks_v2_wire_route: wireRoute,
            normalization_policy: rule.normalization_policy,
          },
          "            ",
        )}\n        );\n    }`,
      );
    }
  }

  return `/// Resolve capability by family rules on the normalized (prefix-stripped) alias.
/// Returns None if no rule matches (caller falls through to provider_fallback).
///
/// Rules are ordered by match_priority descending.
/// Generated from manifest family_rules.
pub fn lookup_by_family_rules(provider: &str, normalized: &str) -> Option<CapabilityResult> {
    let lower = normalized.to_ascii_lowercase();
    let lower = lower.as_str();
    #[allow(clippy::nonminimal_bool)]
${arms.join("\n")}
    None
}`;
}

function buildRustMatchExpr(rule, provider) {
  const allTokens = [rule.match_value, ...(rule.match_aliases ?? [])];
  switch (rule.match_kind) {
    case "exact":
      return allTokens.map((t) => `lower == "${t.toLowerCase()}"`).join(" || ");
    case "prefix":
      return allTokens.map((t) => `lower.starts_with("${t.toLowerCase()}")`).join(" || ");
    case "gpt5-token":
      return allTokens
        .map((t) => `gpt5_token_matches_rs(lower, "${t.toLowerCase()}")`)
        .join(" || ");
    case "gpt5-base":
      return allTokens.map((t) => `gpt5_base_matches_rs(lower, "${t.toLowerCase()}")`).join(" || ");
    case "segment": {
      // Use a contains approach: split on non-alphanumeric, check any segment equals token
      return allTokens
        .map((t) => `lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|s| s == "${t.toLowerCase()}")`)
        .join(" || ");
    }
    case "segment-prefix": {
      return allTokens
        .map(
          (t) =>
            `lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|s| s.starts_with("${t.toLowerCase()}"))`,
        )
        .join(" || ");
    }
    default:
      throw new Error(`unknown match_kind: ${rule.match_kind}`);
  }
}

// Append gpt5 helper fns that the generated code calls
const rustGpt5Helpers = `
// ---------------------------------------------------------------------------
// gpt5 boundary-aware token helpers (used by generated family resolver)
// ---------------------------------------------------------------------------

/// Returns true if \`model\` contains \`token\` at a word boundary (end-of-string or "-").
/// Does not match if followed immediately by a digit or letter.
fn gpt5_token_matches_rs(model: &str, token: &str) -> bool {
    let lower = model;
    let tok_lower = token;
    let mut start = 0;
    loop {
        match lower[start..].find(tok_lower) {
            None => return false,
            Some(rel_idx) => {
                let abs_idx = start + rel_idx;
                let after_idx = abs_idx + tok_lower.len();
                let after_char = lower[after_idx..].chars().next();
                match after_char {
                    None | Some('-') => return true,
                    _ => start = after_idx,
                }
            }
        }
    }
}

/// Like gpt5_token_matches_rs but also rejects short -<1-3 digit> suffixes.
fn gpt5_base_matches_rs(model: &str, token: &str) -> bool {
    let lower = model;
    let tok_lower = token;
    let mut start = 0;
    loop {
        match lower[start..].find(tok_lower) {
            None => return false,
            Some(rel_idx) => {
                let abs_idx = start + rel_idx;
                let after_idx = abs_idx + tok_lower.len();
                let suffix = &lower[after_idx..];
                if suffix.is_empty() {
                    return true;
                }
                if !suffix.starts_with('-') {
                    start = after_idx;
                    continue;
                }
                let dash_rest = &suffix[1..];
                // Reject -<1-3 digits> that look like version numbers.
                let is_short_version = dash_rest
                    .chars()
                    .take(4)
                    .enumerate()
                    .all(|(i, c)| {
                        if i < 3 { c.is_ascii_digit() }
                        else { !c.is_ascii_alphanumeric() }
                    })
                    && dash_rest.chars().next().is_some_and(|c| c.is_ascii_digit());
                if is_short_version {
                    start = after_idx;
                    continue;
                }
                return true;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "generated_model_capabilities_tests.rs"]
mod tests;
`;

const finalRustContent = rustfmt(rustContent + rustGpt5Helpers);

// ---------------------------------------------------------------------------
// TypeScript code generation
// ---------------------------------------------------------------------------

function tsEffortList(efforts) {
  return `[${efforts.map((e) => `"${e}"`).join(", ")}] as const`;
}

function tsEffortOrNull(e) {
  if (e === null || e === undefined) return "null";
  return `"${e}"`;
}

function tsDbv2Route(route) {
  return `"${route}"`;
}

function tsThinkingMode(mode) {
  return `"${mode}"`;
}

function tsNormPolicy(policy) {
  return `"${policy}"`;
}

function emitTsCapabilityResult(r, indent = "  ") {
  const i = indent;
  return [
    `{`,
    `${i}  registryLabel: ${r.registry_label === null ? "null" : `"${r.registry_label}"`},`,
    `${i}  thinkingMode: ${tsThinkingMode(r.thinking_mode)},`,
    `${i}  supportedEfforts: ${tsEffortList(r.supported_efforts)},`,
    `${i}  defaultEffort: ${tsEffortOrNull(r.default_effort)},`,
    `${i}  databricksV2WireRoute: ${tsDbv2Route(r.databricks_v2_wire_route)},`,
    `${i}  normalizationPolicy: ${tsNormPolicy(r.normalization_policy)},`,
    `${i}}`,
  ].join(`\n${i}`);
}

// Build TS exact records map
const tsExactEntries = [];
for (const rec of manifest.exact_records ?? []) {
  const result = resolve(rec.provider, rec.raw_model_id);
  const clean = { ...result };
  delete clean._provenance;
  tsExactEntries.push({ rec, clean });
}

// Build TS family rule resolution (inlined ordered if-chain for readability)
function emitTsFamilyResolver() {
  const sorted = [...manifest.family_rules].sort((a, b) => b.match_priority - a.match_priority);
  const arms = [];
  for (const rule of sorted) {
    for (const provider of rule.providers) {
      // databricks_v2_wire_route is only meaningful for databricks_v2; all other providers get not-applicable
      const wireRoute = provider === "databricks_v2"
        ? rule.databricks_v2_wire_route
        : "not-applicable";
      const clean = {
        registry_label: rule.registry_label ?? null,
        thinking_mode: rule.thinking_mode,
        supported_efforts: rule.supported_efforts,
        default_effort: rule.default_effort,
        databricks_v2_wire_route: wireRoute,
        normalization_policy: rule.normalization_policy,
      };
      const matchExpr = buildTsMatchExpr(rule, provider);
      arms.push(
        `  // rule: ${rule.id}, provider: ${provider}, priority: ${rule.match_priority}\n  if (provider === "${provider}" && (${matchExpr})) {\n    return ${emitTsCapabilityResult(clean, "    ")};\n  }`,
      );
    }
  }
  return arms.join("\n");
}

function buildTsMatchExpr(rule, provider) {
  const allTokens = [rule.match_value, ...(rule.match_aliases ?? [])];
  switch (rule.match_kind) {
    case "exact":
      return allTokens.map((t) => `lower === "${t.toLowerCase()}"`).join(" || ");
    case "prefix":
      return allTokens.map((t) => `lower.startsWith("${t.toLowerCase()}")`).join(" || ");
    case "gpt5-token":
      return allTokens
        .map((t) => `gpt5TokenMatchesGenerated(lower, "${t.toLowerCase()}")`)
        .join(" || ");
    case "gpt5-base":
      return allTokens
        .map((t) => `gpt5BaseMatchesGenerated(lower, "${t.toLowerCase()}")`)
        .join(" || ");
    case "segment":
      return allTokens
        .map(
          (t) =>
            `lower.split(/[^a-z0-9]+/).includes("${t.toLowerCase()}")`,
        )
        .join(" || ");
    case "segment-prefix":
      return allTokens
        .map(
          (t) =>
            `lower.split(/[^a-z0-9]+/).some(s => s.startsWith("${t.toLowerCase()}"))`,
        )
        .join(" || ");
    default:
      throw new Error(`unknown match_kind: ${rule.match_kind}`);
  }
}

const tsProviderFallbacks = providerFallbackKeys
  .map((provider) => {
    const fb = manifest.provider_fallbacks[provider];
    const blankClean = { ...fb.blank, registry_label: null };
    const concClean = { ...fb.concrete_unknown, registry_label: null };
    delete blankClean._provenance;
    delete concClean._provenance;
    return `  "${provider}": {
    blank: ${emitTsCapabilityResult(blankClean, "    ")},
    concreteUnknown: ${emitTsCapabilityResult(concClean, "    ")},
  },`;
  })
  .join("\n");

const defaultFbBlank = { ...manifest.provider_fallbacks["_default"].blank, registry_label: null };
const defaultFbConc = {
  ...manifest.provider_fallbacks["_default"].concrete_unknown,
  registry_label: null,
};
delete defaultFbBlank._provenance;
delete defaultFbConc._provenance;

const tsContent = `// biome-ignore-all format: generated — do not edit by hand.
// Regenerate with: node scripts/generate-model-capabilities.mjs
// Source: scripts/model-capabilities.json
//
// Resolver: provider+rawModelId → exact lookup → family rules → provider fallback (plan v4).
// Not owned here: OpenAI/legacy Databricks/OpenRouter transport; final labels (resolveModelLabel() authoritative).

/** Valid thinking-effort values accepted by buzz-agent (mirrors parse_thinking_effort in config.rs). */
export const THINKING_EFFORT_VALUES = [${VALID_EFFORTS.map((e) => `"${e}"`).join(", ")}] as const;
export type ThinkingEffortValue = (typeof THINKING_EFFORT_VALUES)[number];

/** Databricks v2 wire route. NotApplicable for non-DBv2 providers. */
export type DatabricksV2WireRoute =
  | "openai-responses"
  | "anthropic-messages"
  | "mlflow-chat"
  | "route-unknown"
  | "not-applicable";

/** Anthropic thinking API shape for this model. */
export type ThinkingMode =
  | "manual-budget"
  | "adaptive"
  | "omit-fields"
  | "none"
  | "not-applicable";

/** How to normalize effort values before sending to the provider. */
export type NormalizationPolicy = "none" | "openai-standard" | "openai-clamp-max-to-xhigh";

/** Complete resolved capability record for a (provider, rawModelId) pair. Every axis populated. */
export type CapabilityResult = {
  /** Optional static display label. Feeds resolveModelLabel()'s registry tier only. */
  readonly registryLabel: string | null;
  readonly thinkingMode: ThinkingMode;
  readonly supportedEfforts: ReadonlyArray<ThinkingEffortValue>;
  readonly defaultEffort: ThinkingEffortValue | null;
  readonly databricksV2WireRoute: DatabricksV2WireRoute;
  readonly normalizationPolicy: NormalizationPolicy;
};

/** Valid thinking-effort values accepted by buzz-agent. */
export const BUZZ_AGENT_THINKING_EFFORT_VALUES = THINKING_EFFORT_VALUES;

/** Databricks v2 known model IDs. Mirrors goose DATABRICKS_V2_KNOWN_MODELS. */
export const DATABRICKS_V2_KNOWN_MODELS = [
${manifest.databricks_v2_known_models
  .map((id) => `  "${id}",`)
  .join("\n")}
] as const;

/** Databricks endpoint-ID to display-name registry. Generated from manifest registry_labels section.
 *  Feeds the static registry tier of resolveModelLabel(). */
export const DATABRICKS_MODEL_NAMES: Map<string, string> = new Map([
${registryLabelsArr
  .map(({ id, label }) => `  ["${id}", "${label}"],`)
  .join("\n")}
]);

// ---------------------------------------------------------------------------
// gpt5 boundary-aware token helpers
// ---------------------------------------------------------------------------

function gpt5TokenMatchesGenerated(m: string, token: string): boolean {
  let start = 0;
  while (true) {
    const idx = m.indexOf(token, start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    const afterChar = afterIdx < m.length ? m[afterIdx] : "";
    if (afterChar === "" || afterChar === "-") return true;
    start = afterIdx;
  }
}

function gpt5BaseMatchesGenerated(m: string, token: string): boolean {
  let start = 0;
  while (true) {
    const idx = m.indexOf(token, start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    const suffix = m.slice(afterIdx);
    if (suffix === "") return true;
    if (!suffix.startsWith("-")) { start = afterIdx; continue; }
    const dashRest = suffix.slice(1);
    if (/^\\d{1,3}(?:[^a-z\\d]|$)/i.test(dashRest)) { start = afterIdx; continue; }
    return true;
  }
}

// ---------------------------------------------------------------------------
// Exact records — provider-qualified, pre-prefix-stripping
// ---------------------------------------------------------------------------

const EXACT_RECORDS = new Map<string, CapabilityResult>([
${tsExactEntries
  .map(({ rec, clean }) => {
    return `  ["${rec.provider}::${rec.raw_model_id}", ${emitTsCapabilityResult(clean, "  ")}],`;
  })
  .join("\n")}
]);

// ---------------------------------------------------------------------------
// Provider fallbacks
// ---------------------------------------------------------------------------

const PROVIDER_FALLBACKS: Record<string, { blank: CapabilityResult; concreteUnknown: CapabilityResult }> = {
${tsProviderFallbacks}
};

const DEFAULT_FALLBACK = {
  blank: ${emitTsCapabilityResult(defaultFbBlank, "  ")},
  concreteUnknown: ${emitTsCapabilityResult(defaultFbConc, "  ")},
};

// ---------------------------------------------------------------------------
// Strip catalog prefix — finds first family token occurrence
// ---------------------------------------------------------------------------

export function stripCatalogPrefix(model: string): string {
  const FAMILY_TOKENS = [${manifest.family_tokens.map((t) => `"${t}"`).join(", ")}] as const;
  let firstIdx = Infinity;
  for (const tok of FAMILY_TOKENS) {
    const idx = model.toLowerCase().indexOf(tok);
    if (idx !== -1 && idx < firstIdx) firstIdx = idx;
  }
  return firstIdx === Infinity ? model : model.slice(firstIdx);
}

// ---------------------------------------------------------------------------
// Family rule resolver (generated ordered if-chain)
// ---------------------------------------------------------------------------

function lookupByFamilyRules(provider: string, normalized: string): CapabilityResult | null {
  const lower = normalized.toLowerCase();
${emitTsFamilyResolver()}
  return null;
}

// ---------------------------------------------------------------------------
// Top-level resolve — total function, always returns a complete result
// ---------------------------------------------------------------------------

/**
 * Resolve (provider, rawModelId) → CapabilityResult.
 *
 * Total function — always returns a complete result. Consumers never compose
 * fields from multiple tiers at runtime.
 *
 * Resolution order (plan v4):
 *   1. Provider-qualified raw exact lookup (before prefix stripping)
 *   2. Provider-scoped family rules on normalized alias
 *   3. Per-axis provider fallback (blank vs concrete-unknown)
 */
export function resolveModelCapabilities(
  provider: string,
  rawModelId: string,
): CapabilityResult {
  // Step 1: raw exact lookup
  const exactKey = \`\${provider}::\${rawModelId}\`;
  const exact = EXACT_RECORDS.get(exactKey);
  if (exact) return exact;

  // Step 2: family rules on normalized alias
  const normalized = stripCatalogPrefix(rawModelId);
  const family = lookupByFamilyRules(provider, normalized);
  if (family) return family;

  // Step 3: provider fallback
  const isBlank = rawModelId.trim() === "";
  const fb = PROVIDER_FALLBACKS[provider] ?? DEFAULT_FALLBACK;
  return isBlank ? { ...fb.blank, registryLabel: null } : { ...fb.concreteUnknown, registryLabel: null };
}
`;

// ---------------------------------------------------------------------------
// Write or check files
// ---------------------------------------------------------------------------

const outputs = [
  {
    path: outputDirOverride
      ? join(outputDirOverride, "generated_model_capabilities.rs")
      : join(repoRoot, "crates", "buzz-agent", "src", "generated_model_capabilities.rs"),
    content: finalRustContent,
    label: "Rust",
  },
  {
    path: outputDirOverride
      ? join(outputDirOverride, "modelCapabilities.ts")
      : join(
          repoRoot,
          "desktop",
          "src",
          "features",
          "agents",
          "ui",
          "modelCapabilities.ts",
        ),
    content: tsContent,
    label: "TypeScript",
  },
];

let checkFailed = false;
for (const { path, content, label } of outputs) {
  if (CHECK_MODE) {
    if (!existsSync(path)) {
      console.error(`CHECK FAILED: ${label} file does not exist: ${path}`);
      checkFailed = true;
      continue;
    }
    const existing = readFileSync(path, "utf8");
    if (existing !== content) {
      console.error(`CHECK FAILED: ${label} file is stale: ${path}`);
      console.error("Run: node scripts/generate-model-capabilities.mjs  to regenerate.");
      checkFailed = true;
    } else {
      console.log(`OK: ${label}`);
    }
  } else {
    writeFileSync(path, content, "utf8");
    console.log(`Wrote ${label}: ${path}`);
  }
}

if (CHECK_MODE && checkFailed) {
  process.exit(1);
}
if (!CHECK_MODE) {
  console.log("Done. Generated 3 files.");
}
