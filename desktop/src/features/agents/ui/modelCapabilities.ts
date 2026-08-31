/**
 * Runtime model-capability interpreter (TypeScript).
 *
 * `scripts/model-capabilities.json` is the single source of truth for every
 * model's six-axis capability profile (thinking mode, supported efforts,
 * default effort, Databricks v2 wire route, normalization policy, and picker
 * label). This module imports that manifest and interprets it at runtime,
 * mirroring the Rust interpreter in `crates/buzz-agent/src/model_capabilities.rs`
 * line for line. There is no codegen: both interpreters read the same
 * hand-curated manifest, and the shared normative corpus
 * (`scripts/normative-corpus.json`) is the cross-language contract that
 * guarantees they agree.
 *
 * ## Resolution algorithm (`resolveModelCapabilities`)
 *   1. Provider canonicalization (trim, lowercase, alias map) — done by the
 *      caller via `canonicalizeProvider`.
 *   2. Provider-qualified exact-record lookup (case-insensitive on the id).
 *   3. Boundary-aware family-rule match: strip any endpoint prefix at the first
 *      family token on a non-alphanumeric boundary, then take the longest match
 *      across every rule's `matchValue` and `matchAliases`, breaking ties on the
 *      lexicographically smallest rule id.
 *   4. Provider fallback, distinguishing a blank model id from a
 *      concrete-unknown one.
 *
 * Every path yields a complete six-axis result; `registryLabel` is populated
 * only on an exact-record hit.
 */
import { z } from "zod";
import manifestJson from "@model-capabilities-manifest";

/** Valid thinking-effort values accepted by buzz-agent (mirrors parse_thinking_effort in config.rs). */
export const THINKING_EFFORT_VALUES = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;
export type ThinkingEffortValue = (typeof THINKING_EFFORT_VALUES)[number];

/** Databricks v2 wire route. `not-applicable` for non-DBv2 providers. */
export type DatabricksV2WireRoute =
  | "openai-responses"
  | "anthropic-messages"
  | "mlflow-chat"
  | "route-unknown"
  | "not-applicable";

/** How a model activates and controls reasoning depth on the wire. */
export type ThinkingMode =
  | "adaptive"
  | "manual-budget"
  | "none"
  | "omit-fields";

/** Post-resolution effort normalization applied before a request is sent. */
export type NormalizationPolicy =
  | "none"
  | "openai-standard"
  | "openai-clamp-max-to-xhigh";

/** Complete resolved capability record for a (provider, rawModelId) pair. Every axis populated. */
export type CapabilityResult = {
  readonly thinkingMode: ThinkingMode;
  readonly supportedEfforts: ReadonlyArray<ThinkingEffortValue>;
  readonly defaultEffort: ThinkingEffortValue | null;
  readonly databricksV2WireRoute: DatabricksV2WireRoute;
  readonly normalizationPolicy: NormalizationPolicy;
  /** Static display label. Populated only on a provider-qualified exact-record hit. */
  readonly registryLabel: string | null;
};

// ---------------------------------------------------------------------------
// Manifest schema — runtime-validates the bundled manifest, mirroring the
// strict serde (`deny_unknown_fields` + real enums) + validate_manifest in the
// Rust interpreter. A malformed bundled manifest is a build-time data error
// that must never ship, so parse failure throws.
// ---------------------------------------------------------------------------

const EffortSchema = z.enum(THINKING_EFFORT_VALUES);
const ThinkingModeSchema = z.enum([
  "adaptive",
  "manual-budget",
  "none",
  "omit-fields",
]);
const WireRouteSchema = z.enum([
  "openai-responses",
  "anthropic-messages",
  "mlflow-chat",
  "route-unknown",
  "not-applicable",
]);
const NormalizationSchema = z.enum([
  "none",
  "openai-standard",
  "openai-clamp-max-to-xhigh",
]);

const FamilyRuleSchema = z
  .object({
    id: z.string(),
    match_kind: z.enum(["exact", "prefix"]),
    match_value: z.string(),
    match_aliases: z.array(z.string()).default([]),
    providers: z.array(z.string()),
    thinking_mode: ThinkingModeSchema,
    supported_efforts: z.array(EffortSchema),
    default_effort: EffortSchema.nullable(),
    databricks_v2_wire_route: WireRouteSchema,
    normalization_policy: NormalizationSchema,
    // Documentation-only key; modeled so strict parsing accepts the manifest
    // while still rejecting an unmodeled (typo'd) field. Mirrors the Rust
    // `FamilyRule` doc field under `deny_unknown_fields`.
    _comment: z.string().optional(),
  })
  .strict();

const ExactRecordSchema = z
  .object({
    provider: z.string(),
    raw_model_id: z.string(),
    registry_label: z.string(),
    thinking_mode: ThinkingModeSchema,
    supported_efforts: z.array(EffortSchema),
    default_effort: EffortSchema.nullable(),
    databricks_v2_wire_route: WireRouteSchema,
    normalization_policy: NormalizationSchema,
    // Documentation/provenance keys; modeled for strict parsing, not read at
    // runtime. Mirrors the Rust `ExactRecord` doc fields under
    // `deny_unknown_fields`.
    _provenance: z.string().optional(),
    source: z.string().optional(),
    _source: z.string().optional(),
    _reconciliation: z.string().optional(),
    _reconciliation_note: z.string().optional(),
    _reconciliation_doc: z.string().optional(),
  })
  .strict();

const FallbackStateSchema = z
  .object({
    databricks_v2_wire_route: WireRouteSchema,
    thinking_mode: ThinkingModeSchema,
    supported_efforts: z.array(EffortSchema),
    default_effort: EffortSchema.nullable(),
    normalization_policy: NormalizationSchema,
  })
  .strict();

const FallbackPairSchema = z
  .object({
    blank: FallbackStateSchema,
    concrete_unknown: FallbackStateSchema,
  })
  .strict();

// Fixed named providers with a `_default` catch-all, mirroring the Rust
// `ProviderFallbacks` struct. Enumerating the keys structurally guarantees
// `_default` is present (the resolver's total-function backstop).
const ProviderFallbacksSchema = z
  .object({
    anthropic: FallbackPairSchema,
    openai: FallbackPairSchema,
    databricks: FallbackPairSchema,
    databricks_v2: FallbackPairSchema,
    openrouter: FallbackPairSchema,
    _default: FallbackPairSchema,
  })
  .strict();

export const ManifestSchema = z
  .object({
    family_tokens: z.array(z.string()).min(1),
    label_family_tokens: z.array(z.string()).min(1),
    family_rules: z.array(FamilyRuleSchema),
    databricks_v2_known_models: z.array(z.string()),
    exact_records: z.array(ExactRecordSchema),
    provider_fallbacks: ProviderFallbacksSchema,
    // Root documentation keys; modeled for strict parsing, not read at runtime.
    // Mirrors the Rust `Manifest` doc fields under `deny_unknown_fields`.
    _comment: z.string().optional(),
    _comment_label_family_tokens: z.string().optional(),
    _comment_databricks_v2_known_models: z.string().optional(),
    _sources: z.record(z.string(), z.string()).optional(),
  })
  .strict();

type ParsedManifest = z.infer<typeof ManifestSchema>;
type FamilyRule = z.infer<typeof FamilyRuleSchema>;
type FallbackState = z.infer<typeof FallbackStateSchema>;
type FallbackPair = z.infer<typeof FallbackPairSchema>;

const MANIFEST: ParsedManifest = ManifestSchema.parse(manifestJson);

// Prototype-safe provider→fallback lookup. A plain-object index would return
// `Object.prototype.constructor` / `Object.prototype` for the adversarial
// providers `constructor` / `__proto__`, defeating the `_default` catch-all;
// a Map keys only on real entries. Mirrors `ProviderFallbacks::get`.
const PROVIDER_FALLBACKS: ReadonlyMap<string, FallbackPair> = new Map(
  Object.entries(MANIFEST.provider_fallbacks),
);

function fallbackPair(canon: string): FallbackPair {
  // `_default` is a required key on `ProviderFallbacksSchema`, so referencing
  // it directly is typed as a non-optional `FallbackPair` — no assertion, and
  // the total-function backstop is guaranteed by the schema, not by `!`.
  return PROVIDER_FALLBACKS.get(canon) ?? MANIFEST.provider_fallbacks._default;
}

// ---------------------------------------------------------------------------
// Provider canonicalization
// ---------------------------------------------------------------------------

const PROVIDER_ALIASES = new Map<string, string>([
  ["openai-compat", "openai"],
  ["databricks-v2", "databricks_v2"],
]);

/** Canonicalize a provider name: trim, lowercase, apply the alias map. */
export function canonicalizeProvider(provider: string): string {
  const canon = provider.trim().toLowerCase();
  return PROVIDER_ALIASES.get(canon) ?? canon;
}

// ---------------------------------------------------------------------------
// Boundary-aware prefix helpers — mirror strip_catalog_prefix / prefix_matches.
// ---------------------------------------------------------------------------

function isAsciiAlphanumeric(ch: string): boolean {
  return /^[a-z0-9]$/i.test(ch);
}

/**
 * Strip an endpoint-naming prefix by locating the earliest family token that
 * begins on a non-alphanumeric boundary (or at the start), returning the slice
 * from that token onward. Returns the input unchanged when no token qualifies.
 */
export function stripCatalogPrefix(
  modelLower: string,
  familyTokens: ReadonlyArray<string>,
): string {
  let best = Number.POSITIVE_INFINITY;
  for (const tok of familyTokens) {
    let from = 0;
    while (true) {
      const idx = modelLower.indexOf(tok, from);
      if (idx === -1) break;
      if (idx === 0 || !isAsciiAlphanumeric(modelLower[idx - 1])) {
        if (idx < best) best = idx;
        break;
      }
      from = idx + 1;
    }
  }
  return best === Number.POSITIVE_INFINITY
    ? modelLower
    : modelLower.slice(best);
}

/**
 * Boundary-aware prefix test: `s` equals `token`, or `s` starts with `token`
 * and the following character is a non-alphanumeric boundary.
 */
function prefixMatches(token: string, s: string): boolean {
  if (!s.startsWith(token)) return false;
  const rest = s.slice(token.length);
  return rest.length === 0 || !isAsciiAlphanumeric(rest[0]);
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

function toResult(
  axes: FamilyRule | FallbackState,
  route: DatabricksV2WireRoute,
  registryLabel: string | null,
): CapabilityResult {
  return {
    thinkingMode: axes.thinking_mode,
    supportedEfforts: axes.supported_efforts,
    defaultEffort: axes.default_effort,
    databricksV2WireRoute: route,
    normalizationPolicy: axes.normalization_policy,
    registryLabel,
  };
}

function isDatabricksModelServiceFqn(model: string): boolean {
  const components = model.split(".");
  return (
    components.length === 3 &&
    components.every(
      (component) =>
        component.length > 0 &&
        !/\s/.test(component) &&
        !component.includes("/"),
    )
  );
}

/**
 * Resolve the capability profile for a `(provider, rawModelId)` pair.
 *
 * Total function — always returns a complete result. Provider canonicalization
 * happens inside the resolver (trim, lowercase, alias map), so callers pass raw
 * provider names. Mirrors `resolve` in the Rust interpreter exactly.
 */
export function resolveModelCapabilities(
  provider: string,
  rawModelId: string,
): CapabilityResult {
  const canon = canonicalizeProvider(provider);
  const blank = rawModelId.trim().length === 0;
  // Unity Catalog FQNs are neutral model-service identities. Resolve them
  // through the concrete-unknown fallback before suffix family matching.
  const modelServiceFqn =
    canon === "databricks_v2" && isDatabricksModelServiceFqn(rawModelId);

  // 1. Provider-qualified exact-record lookup (case-insensitive on the id).
  if (!blank && !modelServiceFqn) {
    const idLower = rawModelId.toLowerCase();
    for (const rec of MANIFEST.exact_records) {
      if (
        rec.provider === canon &&
        rec.raw_model_id.toLowerCase() === idLower
      ) {
        return toResult(rec, rec.databricks_v2_wire_route, rec.registry_label);
      }
    }
  }

  // 2. Boundary-aware family match: longest token wins, lexicographic tie-break.
  if (!blank && !modelServiceFqn) {
    const modelLower = rawModelId.toLowerCase();
    const stripped = stripCatalogPrefix(modelLower, MANIFEST.family_tokens);
    let best: { len: number; rule: FamilyRule } | null = null;
    for (const rule of MANIFEST.family_rules) {
      if (!rule.providers.includes(canon)) continue;
      let matched: number | null = null;
      for (const tok of [rule.match_value, ...rule.match_aliases]) {
        const ok =
          rule.match_kind === "exact"
            ? stripped === tok
            : prefixMatches(tok, stripped);
        if (ok)
          matched =
            matched === null ? tok.length : Math.max(matched, tok.length);
      }
      if (matched !== null) {
        const better =
          best === null ||
          matched > best.len ||
          (matched === best.len && rule.id < best.rule.id);
        if (better) best = { len: matched, rule };
      }
    }
    if (best !== null) {
      const route: DatabricksV2WireRoute =
        canon === "databricks_v2"
          ? best.rule.databricks_v2_wire_route
          : "not-applicable";
      return toResult(best.rule, route, null);
    }
  }

  // 3. Provider fallback (blank vs. concrete-unknown); never carries a label.
  const pair = fallbackPair(canon);
  const state = blank ? pair.blank : pair.concrete_unknown;
  return toResult(state, state.databricks_v2_wire_route, null);
}

/** Authoritative list of known Databricks v2 model ids, sourced from the manifest. */
export const DATABRICKS_V2_KNOWN_MODELS: ReadonlyArray<string> =
  MANIFEST.databricks_v2_known_models;

export type RegistryLabelRecord = {
  readonly provider: string;
  readonly raw_model_id: string;
  readonly registry_label: string;
};

export function databricksRegistryLabelForRecords(
  rawModelId: string,
  records: ReadonlyArray<RegistryLabelRecord>,
  familyTokens: ReadonlyArray<string>,
): string | null {
  if (!rawModelId.trim()) return null;

  const idLower = rawModelId.toLowerCase();
  const exact = records.find(
    (rec) =>
      rec.provider === "databricks_v2" &&
      rec.raw_model_id.toLowerCase() === idLower,
  );
  if (exact) return exact.registry_label;

  const strippedQuery = stripCatalogPrefix(idLower, familyTokens);
  if (strippedQuery === idLower) return null;
  let matchingRecord: RegistryLabelRecord | null = null;
  for (const rec of records) {
    if (rec.provider !== "databricks_v2") continue;
    const strippedRecord = stripCatalogPrefix(
      rec.raw_model_id.toLowerCase(),
      familyTokens,
    );
    if (strippedRecord === strippedQuery) {
      if (matchingRecord) return null;
      matchingRecord = rec;
    }
  }
  return matchingRecord?.registry_label ?? null;
}

export function databricksRegistryLabel(rawModelId: string): string | null {
  return databricksRegistryLabelForRecords(
    rawModelId,
    MANIFEST.exact_records,
    MANIFEST.label_family_tokens,
  );
}
