import {
  DATABRICKS_MODEL_NAMES,
  resolveModelCapabilities,
} from "../ui/modelCapabilities.ts";

/**
 * Known provider-id aliases that must be normalized before generated lookups.
 *
 * - "databricks-v2" (hyphen form) → "databricks_v2" (underscore form used in manifest).
 * - "openai-compat" → "openai": Rust already accepts "openai-compat" as Provider::OpenAi
 *   (crates/buzz-agent/src/config.rs); TS must canonicalize identically so the UI shows
 *   the same effort table that the Rust request path will apply.
 *
 * Map is used (not a plain object) to avoid prototype-chain collisions
 * ("constructor", "__proto__", etc.) silently resolving to a built-in value.
 */
const PROVIDER_ALIASES = new Map<string, string>([
  ["databricks-v2", "databricks_v2"],
  ["openai-compat", "openai"],
]);

/**
 * Normalizes a provider id to the canonical form expected by the generated
 * manifest: lowercases, trims, and applies the known alias table.
 *
 * Used as the single canonicalization point before every generated lookup
 * in both resolveModelLabel() and getProviderEffortConfig().
 */
export function canonicalizeProvider(provider: string): string {
  const normalized = provider.trim().toLowerCase();
  return PROVIDER_ALIASES.get(normalized) ?? normalized;
}

/**
 * Resolves a human-readable label for a model, following the three-tier
 * precedence documented in AGENTS.md:
 *
 *   1. Nonblank discovered/API name (e.g. from AgentModelInfo.name)
 *   2. Registry lookup by ID:
 *      - When `provider` is supplied: provider-qualified exact record only.
 *        If the generated lookup returns no registryLabel, return the raw ID.
 *        The unscoped DATABRICKS_MODEL_NAMES registry is NOT consulted for a
 *        known provider — this prevents Databricks names from leaking through
 *        anthropic/openai provider contexts.
 *      - When `provider` is absent/null: unscoped DATABRICKS_MODEL_NAMES map.
 *   3. Raw ID unchanged
 *
 * Returns the empty string when both id and discoveredName are blank.
 * Use formatAgentModelLabel() when a null/empty id should render "Auto".
 */
export function resolveModelLabel(
  id: string,
  discoveredName?: string | null | undefined,
  provider?: string | null | undefined,
): string {
  const trimmedName = discoveredName?.trim();
  if (trimmedName) return trimmedName;
  const trimmedId = id.trim();
  if (!trimmedId) return "";
  // Provider-qualified exact record (registry_label tier, provider-scoped).
  // When a provider is known, do NOT fall through to the unscoped registry —
  // return the raw ID directly on a miss (P3-B contract).
  if (provider?.trim()) {
    const canonical = canonicalizeProvider(provider);
    const registryLabel = resolveModelCapabilities(
      canonical,
      trimmedId,
    ).registryLabel;
    return registryLabel ?? trimmedId;
  }
  // Providerless path only: unscoped registry map for legacy/inherited IDs.
  return DATABRICKS_MODEL_NAMES.get(trimmedId) ?? trimmedId;
}

/**
 * Returns a human-readable model label for an agent or persona, falling back
 * to "Auto" when no model is set (empty or whitespace-only).
 *
 * For known Databricks managed endpoints the registry-curated name is returned
 * (e.g. "databricks-gpt-5-5" → "GPT-5.5"). Unknown or custom endpoint IDs are
 * returned unchanged — no heuristic string mangling.
 *
 * Pass `provider` when the inference provider is known to get a provider-qualified
 * registry label (e.g. exact records in the generated manifest take priority).
 */
export function formatAgentModelLabel(
  model: string | null | undefined,
  provider?: string | null | undefined,
) {
  const trimmed = model?.trim();
  if (!trimmed) return "Auto";
  return resolveModelLabel(trimmed, null, provider);
}
