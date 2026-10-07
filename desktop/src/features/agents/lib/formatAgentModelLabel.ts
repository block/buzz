import {
  canonicalizeProvider,
  databricksRegistryLabel,
  resolveModelCapabilities,
} from "../ui/modelCapabilities";

// Re-exported so the label surface remains the single import site for provider
// canonicalization; the interpreter owns the alias map.
export { canonicalizeProvider };

/**
 * Resolves a human-readable label for a model, following a three-tier
 * precedence:
 *
 *   1. Non-blank discovered/API name (e.g. from `AgentModelInfo.name`) that is
 *      genuinely distinct from the id. A discovered name that merely echoes the
 *      trimmed id carries no display information, so it is treated as absent and
 *      falls through to the registry tier — this covers buzz-agent's Databricks
 *      discovery contract (`{id, name: id}`) and any harness/version skew that
 *      echoes the id as the name.
 *   2. Registry lookup by id:
 *      - `provider` supplied → Databricks v2 uses alias-aware exact records;
 *        every other provider uses provider-qualified exact records. On a miss
 *        the raw id is returned; the providerless registry tier is NOT
 *        consulted, so a Databricks endpoint id never leaks a curated label
 *        through an anthropic/openai provider context (the P3-B contract).
 *      - `provider` absent → alias-aware lookup over `databricks_v2` exact
 *        records, for legacy/inherited ids with no provider on hand.
 *   3. Raw id unchanged.
 *
 * Returns the empty string when both id and discoveredName are blank; use
 * `formatAgentModelLabel` when a null/empty id should render "Auto".
 *
 * `resolveModelCapabilities` canonicalizes the provider internally. The
 * providerless registry lookup applies the same family-token stripping and
 * unique-match guard as buzz-agent discovery; only unique exact-record aliases
 * get a label.
 */
export function resolveModelLabel(
  id: string,
  discoveredName?: string | null | undefined,
  provider?: string | null | undefined,
): string {
  const trimmedName = discoveredName?.trim();
  const trimmedId = id.trim();
  // A discovered name distinct from the id is authoritative (tier 1). A name
  // that merely echoes the id is treated as absent so the registry tier runs.
  if (trimmedName && trimmedName !== trimmedId) return trimmedName;
  if (!trimmedId) return "";
  if (provider?.trim()) {
    // Provider-qualified exact-record tier (provider-scoped, no providerless fallback).
    const canonicalProvider = canonicalizeProvider(provider);
    const registryLabel =
      canonicalProvider === "databricks_v2"
        ? databricksRegistryLabel(trimmedId)
        : resolveModelCapabilities(provider, trimmedId).registryLabel;
    return registryLabel ?? trimmedId;
  }
  // Providerless path: alias-aware lookup for legacy/inherited ids.
  return databricksRegistryLabel(trimmedId) ?? trimmedId;
}

/**
 * Returns a human-readable model label for an agent or persona, falling back to
 * "Auto" when no model is set (empty or whitespace-only).
 *
 * For known Databricks managed endpoints the registry-curated name is returned
 * (e.g. "databricks-gpt-5-5" → "GPT-5.5"). Unknown or custom endpoint ids are
 * returned unchanged — no heuristic string mangling. Pass `provider` when the
 * inference provider is known to get a provider-qualified registry label.
 */
export function formatAgentModelLabel(
  model: string | null | undefined,
  provider?: string | null | undefined,
) {
  const trimmed = model?.trim();
  if (!trimmed) return "Auto";
  return resolveModelLabel(trimmed, null, provider);
}
