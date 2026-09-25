import {
  canonicalizeProvider,
  databricksRegistryLabel,
  isDatabricksModelServiceFqn,
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
 *      - `provider` supplied → Databricks v2 uses alias-aware exact records,
 *        then the generative Databricks label grammar; every other provider
 *        uses provider-qualified exact records. On a miss
 *        the raw id is returned; the providerless registry tier is NOT
 *        consulted, so a Databricks endpoint id never leaks a curated label
 *        through an anthropic/openai provider context (the P3-B contract).
 *      - `provider` absent → alias-aware lookup over `databricks_v2` exact
 *        records only (no generated labels), for legacy/inherited ids with no
 *        provider on hand.
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
  // Providerless path: curated alias-aware lookup for legacy/inherited ids.
  // Generated labels need an explicit databricks_v2 provider.
  return databricksRegistryLabel(trimmedId, { generate: false }) ?? trimmedId;
}

/**
 * Returns a human-readable model label for an agent or persona, falling back to
 * "Auto" when no model is set (empty or whitespace-only).
 *
 * For known Databricks managed endpoints the registry-curated name is returned
 * (e.g. "databricks-gpt-5-5" → "GPT-5.5"); with a `databricks_v2` provider, an
 * uncurated endpoint id gets a label parsed strictly from its own tokens.
 * Anything else is returned unchanged. Pass `provider` when the
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

const DATABRICKS_WRAPPERS = ["databricks-", "goose-", "kgoose-", "builderbot-"];

/** Where a Databricks id comes from: its UC `catalog.schema`, else its wrapper. */
function databricksModelSource(id: string): string {
  if (isDatabricksModelServiceFqn(id)) {
    return id.split(".").slice(0, 2).join(".");
  }
  const lower = id.toLowerCase();
  const wrapper = DATABRICKS_WRAPPERS.find((w) => lower.startsWith(w));
  return wrapper ? wrapper.slice(0, -1) : id;
}

function collidingLabels(rows: ReadonlyArray<{ id: string; label: string }>) {
  const counts = new Map<string, number>();
  for (const { label } of rows) {
    const key = label.toLowerCase();
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return (label: string) => (counts.get(label.toLowerCase()) ?? 0) > 1;
}

/**
 * Tells apart Databricks rows whose humanized labels collide (the same model
 * served from several catalogs or wrappers) by suffixing the source:
 * `GPT-6 Astra (system.ai)`, `GPT-5.5 (goose)`. Rows still colliding after
 * that get the full id. Raw-id rows, unique labels, the default `""` row, and
 * other providers are unchanged. Presentation only; ids are never touched.
 */
export function disambiguateModelLabels<
  T extends { id: string; label: string },
>(rows: ReadonlyArray<T>, provider: string | null | undefined): T[] {
  if (canonicalizeProvider(provider ?? "") !== "databricks_v2") {
    return [...rows];
  }
  const humanized = (row: T) => row.id !== "" && row.label !== row.id;
  const collides = collidingLabels(rows.filter(humanized));
  const suffixed = rows.map((row) =>
    humanized(row) && collides(row.label)
      ? { ...row, label: `${row.label} (${databricksModelSource(row.id)})` }
      : row,
  );
  const stillCollides = collidingLabels(
    suffixed.filter((row, i) => row !== rows[i]),
  );
  return suffixed.map((row, i) =>
    row !== rows[i] && stillCollides(row.label)
      ? { ...row, label: `${rows[i].label} (${row.id})` }
      : row,
  );
}
