// biome-ignore-all format: generated — do not edit by hand.
// Regenerate with: node scripts/generate-model-capabilities.mjs
// Source: scripts/model-capabilities.json
//
// Pricing axis: exact (authority, model) lookup for cost estimation.
// Consumers: agent-usage aggregation (Phase 4 of Usage v2 plan).

// ---------------------------------------------------------------------------
// Pricing axis — exact (authority, model) lookup
// ---------------------------------------------------------------------------

/** Per-model token pricing, USD per million tokens. null = price not published. */
export type ModelPricing = {
  /** Input (fresh, non-cache) token price, USD/MTok. */
  readonly inputUsdPerMtok: number;
  /** Output token price, USD/MTok. */
  readonly outputUsdPerMtok: number;
  /** Cache-read token price, USD/MTok. null when provider has not published it. */
  readonly cacheReadUsdPerMtok: number | null;
  /**
   * Cache-write (cache creation) token price, USD/MTok. null when not published.
   * Scoped to the default ephemeral cache class.
   */
  readonly cacheWriteUsdPerMtok: number | null;
};

/**
 * Composite key for pricing lookup: `${authority}\0${model}` (exact strings, no transforms).
 *
 * authority — registered billing-authority token (e.g. "api.anthropic.com").
 * model     — exact model string from the provider API response body.
 */
const PRICING_TABLE: Map<string, ModelPricing> = new Map([
  ["api.anthropic.com\u0000claude-fable-5", {
      inputUsdPerMtok: 10,
      outputUsdPerMtok: 50,
      cacheReadUsdPerMtok: 1,
      cacheWriteUsdPerMtok: 12.5,
    }],
  ["api.anthropic.com\u0000claude-sonnet-5", {
      inputUsdPerMtok: 2,
      outputUsdPerMtok: 10,
      cacheReadUsdPerMtok: 0.2,
      cacheWriteUsdPerMtok: 2.5,
    }],
  ["api.anthropic.com\u0000claude-opus-5", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 25,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.anthropic.com\u0000claude-opus-4-8", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 25,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.anthropic.com\u0000claude-opus-4-7", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 25,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.anthropic.com\u0000claude-opus-4-6", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 25,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.anthropic.com\u0000claude-opus-4-5", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 25,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.anthropic.com\u0000claude-sonnet-4-6", {
      inputUsdPerMtok: 3,
      outputUsdPerMtok: 15,
      cacheReadUsdPerMtok: 0.3,
      cacheWriteUsdPerMtok: 3.75,
    }],
  ["api.anthropic.com\u0000claude-sonnet-4-5", {
      inputUsdPerMtok: 3,
      outputUsdPerMtok: 15,
      cacheReadUsdPerMtok: 0.3,
      cacheWriteUsdPerMtok: 3.75,
    }],
  ["api.anthropic.com\u0000claude-haiku-4-5", {
      inputUsdPerMtok: 1,
      outputUsdPerMtok: 5,
      cacheReadUsdPerMtok: 0.1,
      cacheWriteUsdPerMtok: 1.25,
    }],
  ["api.anthropic.com\u0000claude-opus-4-1", {
      inputUsdPerMtok: 15,
      outputUsdPerMtok: 75,
      cacheReadUsdPerMtok: 1.5,
      cacheWriteUsdPerMtok: 18.75,
    }],
  ["api.openai.com\u0000gpt-5-pro", {
      inputUsdPerMtok: 15,
      outputUsdPerMtok: 120,
      cacheReadUsdPerMtok: null,
      cacheWriteUsdPerMtok: null,
    }],
  ["api.openai.com\u0000gpt-5.6", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 30,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.openai.com\u0000gpt-5.6-sol", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 30,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: 6.25,
    }],
  ["api.openai.com\u0000gpt-5.6-luna", {
      inputUsdPerMtok: 0.2,
      outputUsdPerMtok: 1.2,
      cacheReadUsdPerMtok: 0.02,
      cacheWriteUsdPerMtok: 0.25,
    }],
  ["api.openai.com\u0000gpt-5.6-terra", {
      inputUsdPerMtok: 2,
      outputUsdPerMtok: 12,
      cacheReadUsdPerMtok: 0.2,
      cacheWriteUsdPerMtok: 2.5,
    }],
  ["api.openai.com\u0000gpt-5.5", {
      inputUsdPerMtok: 5,
      outputUsdPerMtok: 30,
      cacheReadUsdPerMtok: 0.5,
      cacheWriteUsdPerMtok: null,
    }],
  ["api.openai.com\u0000gpt-5.4", {
      inputUsdPerMtok: 2.5,
      outputUsdPerMtok: 15,
      cacheReadUsdPerMtok: 0.25,
      cacheWriteUsdPerMtok: null,
    }],
  ["api.openai.com\u0000gpt-5.1", {
      inputUsdPerMtok: 1.25,
      outputUsdPerMtok: 10,
      cacheReadUsdPerMtok: 0.125,
      cacheWriteUsdPerMtok: null,
    }],
  ["api.openai.com\u0000gpt-5", {
      inputUsdPerMtok: 1.25,
      outputUsdPerMtok: 10,
      cacheReadUsdPerMtok: 0.125,
      cacheWriteUsdPerMtok: null,
    }],
]);

/**
 * Look up pricing for an exact (billing authority, model) pair.
 *
 * Both arguments are matched exactly as supplied — no case normalization.
 * The caller must supply the exact registered authority token and the exact
 * provider-API model string. Returns `null` for unrecognised pairs — callers
 * MUST treat null as "price unknown", never as zero cost. There is no family,
 * prefix, or case-folding fallback.
 *
 * @param authority - Registered billing-authority token, e.g. "api.anthropic.com".
 *   This is NOT the runtime provider name; it is the proven billing namespace.
 * @param model - Exact model string returned by the provider API response body.
 */
export function lookupModelPricing(authority: string, model: string): ModelPricing | null {
  const key = `${authority}\0${model}`;
  return PRICING_TABLE.get(key) ?? null;
}
