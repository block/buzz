// biome-ignore-all format: generated — do not edit by hand.
// Regenerate with: node scripts/generate-model-capabilities.mjs
// Source: scripts/model-capabilities.json
//
// Resolver: provider+rawModelId → exact lookup → family rules → provider fallback (plan v4).
// Not owned here: OpenAI/legacy Databricks/OpenRouter transport; final labels (resolveModelLabel() authoritative).

/** Valid thinking-effort values accepted by buzz-agent (mirrors parse_thinking_effort in config.rs). */
export const THINKING_EFFORT_VALUES = ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const;
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
  "databricks-gpt-5-5",
  "databricks-claude-opus-4-7",
] as const;

/** Databricks endpoint-ID to display-name registry. Generated from manifest exact_records
 *  (provider = databricks_v2, registry_label present). Feeds the static registry tier of
 *  resolveModelLabel(). */
export const DATABRICKS_MODEL_NAMES: Map<string, string> = new Map([
  ["databricks-gpt-5-4-mini", "GPT-5.4 mini"],
  ["databricks-gpt-5-4-nano", "GPT-5.4 nano"],
  ["databricks-gpt-5-6-sol", "GPT-5.6 Sol"],
  ["databricks-gpt-5-5", "GPT-5.5"],
  ["databricks-claude-opus-4-7", "Claude Opus 4.7"],
  ["databricks-gpt-5-6-luna", "GPT-5.6 Luna"],
  ["databricks-gpt-5-6-terra", "GPT-5.6 Terra"],
  ["databricks-claude-haiku-4-5", "Claude Haiku 4.5 (latest)"],
  ["databricks-claude-opus-4-1", "Claude Opus 4.1 (latest)"],
  ["databricks-claude-opus-4-5", "Claude Opus 4.5 (latest)"],
  ["databricks-claude-opus-4-6", "Claude Opus 4.6"],
  ["databricks-claude-sonnet-4", "Claude Sonnet 4.5"],
  ["databricks-claude-sonnet-4-5", "Claude Sonnet 4.5 (latest)"],
  ["databricks-claude-sonnet-4-6", "Claude Sonnet 4.6"],
  ["databricks-gemini-2-5-flash", "Gemini 2.5 Flash"],
  ["databricks-gemini-2-5-pro", "Gemini 2.5 Pro"],
  ["databricks-gemini-3-1-flash-lite", "Gemini 3.1 Flash Lite Preview"],
  ["databricks-gemini-3-1-pro", "Gemini 3.1 Pro Preview Custom Tools"],
  ["databricks-gemini-3-flash", "Gemini 3 Flash Preview"],
  ["databricks-gemini-3-pro", "Gemini 3 Pro Preview"],
  ["databricks-glm-5-2", "GLM-5.2"],
  ["databricks-gpt-5", "GPT-5"],
  ["databricks-gpt-5-1", "GPT-5.1"],
  ["databricks-gpt-5-2", "GPT-5.2"],
  ["databricks-gpt-5-4", "GPT-5.4"],
  ["databricks-gpt-5-mini", "GPT-5 Mini"],
  ["databricks-gpt-5-nano", "GPT-5 Nano"],
  ["databricks-gpt-oss-120b", "GPT OSS 120B"],
  ["databricks-gpt-oss-20b", "GPT OSS 20B"],
  ["databricks-kimi-k2-7-code", "Kimi K2.7 Code"],
]);

// ---------------------------------------------------------------------------
// gpt5 boundary-aware token helpers
// ---------------------------------------------------------------------------

function hasLeftBoundaryGenerated(m: string, idx: number): boolean {
  if (idx === 0) return true;
  const prev = m[idx - 1];
  return prev === "-" || prev === ".";
}

function gpt5TokenMatchesGenerated(m: string, token: string): boolean {
  let start = 0;
  while (true) {
    const idx = m.indexOf(token, start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    if (!hasLeftBoundaryGenerated(m, idx)) { start = afterIdx; continue; }
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
    if (!hasLeftBoundaryGenerated(m, idx)) { start = afterIdx; continue; }
    const suffix = m.slice(afterIdx);
    if (suffix === "") return true;
    if (!suffix.startsWith("-")) { start = afterIdx; continue; }
    const dashRest = suffix.slice(1);
    if (/^\d{1,3}(?:[^a-z\d]|$)/i.test(dashRest)) { start = afterIdx; continue; }
    return true;
  }
}

/**
 * gpt-version-segment match: token is an exact segment AND (token itself starts with a digit,
 * OR the next segment after it starts with a digit). Prevents "gpt-neox-20b" from matching
 * "gpt" because "neox" starts with a letter, while "gpt-5.5" matches because next seg "5" is digit.
 */
function gptVersionSegmentMatchesGenerated(m: string, token: string): boolean {
  const segs = m.split(/[^a-z0-9]+/);
  for (let i = 0; i < segs.length; i++) {
    if (segs[i] === token) {
      // Dashless numeric form (e.g. "gpt5") or token itself starts with digit
      if (token.length > 3 || /^\d/.test(token)) return true;
      // For "gpt": require the next segment to start with a digit
      const nextSeg = segs[i + 1];
      if (nextSeg !== undefined && /^\d/.test(nextSeg)) return true;
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// Exact records — provider-qualified, pre-prefix-stripping
// ---------------------------------------------------------------------------

const EXACT_RECORDS = new Map<string, CapabilityResult>([
  ["databricks_v2::databricks-gpt-5-4-mini", {
      registryLabel: "GPT-5.4 mini",
      thinkingMode: "none",
      supportedEfforts: ["low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-4-nano", {
      registryLabel: "GPT-5.4 nano",
      thinkingMode: "none",
      supportedEfforts: ["low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-6-sol", {
      registryLabel: "GPT-5.6 Sol",
      thinkingMode: "none",
      supportedEfforts: ["low", "medium", "high", "max"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-5", {
      registryLabel: "GPT-5.5",
      thinkingMode: "none",
      supportedEfforts: ["low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-claude-opus-4-7", {
      registryLabel: "Claude Opus 4.7",
      thinkingMode: "adaptive",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-gpt-5-6-luna", {
      registryLabel: "GPT-5.6 Luna",
      thinkingMode: "none",
      supportedEfforts: ["low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-6-terra", {
      registryLabel: "GPT-5.6 Terra",
      thinkingMode: "none",
      supportedEfforts: ["low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-claude-haiku-4-5", {
      registryLabel: "Claude Haiku 4.5 (latest)",
      thinkingMode: "omit-fields",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-claude-opus-4-1", {
      registryLabel: "Claude Opus 4.1 (latest)",
      thinkingMode: "omit-fields",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-claude-opus-4-5", {
      registryLabel: "Claude Opus 4.5 (latest)",
      thinkingMode: "manual-budget",
      supportedEfforts: ["low", "medium", "high"] as const,
      defaultEffort: null,
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-claude-opus-4-6", {
      registryLabel: "Claude Opus 4.6",
      thinkingMode: "adaptive",
      supportedEfforts: ["low", "medium", "high", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-claude-sonnet-4", {
      registryLabel: "Claude Sonnet 4.5",
      thinkingMode: "omit-fields",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-claude-sonnet-4-5", {
      registryLabel: "Claude Sonnet 4.5 (latest)",
      thinkingMode: "omit-fields",
      supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-claude-sonnet-4-6", {
      registryLabel: "Claude Sonnet 4.6",
      thinkingMode: "adaptive",
      supportedEfforts: ["low", "medium", "high", "max"] as const,
      defaultEffort: "high",
      databricksV2WireRoute: "anthropic-messages",
      normalizationPolicy: "none",
    }],
  ["databricks_v2::databricks-gemini-2-5-flash", {
      registryLabel: "Gemini 2.5 Flash",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gemini-2-5-pro", {
      registryLabel: "Gemini 2.5 Pro",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gemini-3-1-flash-lite", {
      registryLabel: "Gemini 3.1 Flash Lite Preview",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gemini-3-1-pro", {
      registryLabel: "Gemini 3.1 Pro Preview Custom Tools",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gemini-3-flash", {
      registryLabel: "Gemini 3 Flash Preview",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gemini-3-pro", {
      registryLabel: "Gemini 3 Pro Preview",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-glm-5-2", {
      registryLabel: "GLM-5.2",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gpt-5", {
      registryLabel: "GPT-5",
      thinkingMode: "none",
      supportedEfforts: ["minimal", "low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-1", {
      registryLabel: "GPT-5.1",
      thinkingMode: "none",
      supportedEfforts: ["none", "low", "medium", "high"] as const,
      defaultEffort: "none",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-2", {
      registryLabel: "GPT-5.2",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gpt-5-4", {
      registryLabel: "GPT-5.4",
      thinkingMode: "none",
      supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-mini", {
      registryLabel: "GPT-5 Mini",
      thinkingMode: "none",
      supportedEfforts: ["minimal", "low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-5-nano", {
      registryLabel: "GPT-5 Nano",
      thinkingMode: "none",
      supportedEfforts: ["minimal", "low", "medium", "high"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "openai-responses",
      normalizationPolicy: "openai-standard",
    }],
  ["databricks_v2::databricks-gpt-oss-120b", {
      registryLabel: "GPT OSS 120B",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-gpt-oss-20b", {
      registryLabel: "GPT OSS 20B",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
  ["databricks_v2::databricks-kimi-k2-7-code", {
      registryLabel: "Kimi K2.7 Code",
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "mlflow-chat",
      normalizationPolicy: "openai-clamp-max-to-xhigh",
    }],
]);

// ---------------------------------------------------------------------------
// Provider fallbacks
// ---------------------------------------------------------------------------

const PROVIDER_FALLBACKS = new Map<string, { blank: CapabilityResult; concreteUnknown: CapabilityResult }>([
  ["anthropic", {
    blank: {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        },
    concreteUnknown: {
          registryLabel: null,
          thinkingMode: "omit-fields",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        },
  }],
  ["openai", {
    blank: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        },
    concreteUnknown: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        },
  }],
  ["databricks_v2", {
    blank: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "route-unknown",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        },
    concreteUnknown: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "mlflow-chat",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        },
  }],
  ["databricks", {
    blank: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        },
    concreteUnknown: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        },
  }],
  ["openrouter", {
    blank: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        },
    concreteUnknown: {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        },
  }],
]);

const DEFAULT_FALLBACK: { blank: CapabilityResult; concreteUnknown: CapabilityResult } = {
  blank: {
      registryLabel: null,
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "not-applicable",
      normalizationPolicy: "none",
    },
  concreteUnknown: {
      registryLabel: null,
      thinkingMode: "none",
      supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh", "max"] as const,
      defaultEffort: "medium",
      databricksV2WireRoute: "not-applicable",
      normalizationPolicy: "none",
    },
};

// ---------------------------------------------------------------------------
// Strip catalog prefix — boundary-aware, finds first boundary-aligned family token
// ---------------------------------------------------------------------------

export function stripCatalogPrefix(model: string): string {
  const FAMILY_TOKENS = ["claude-", "gpt-"] as const;
  const lower = model.toLowerCase();
  let firstIdx = Infinity;
  for (const tok of FAMILY_TOKENS) {
    let start = 0;
    while (true) {
      const idx = lower.indexOf(tok, start);
      if (idx === -1) break;
      // Boundary check: position 0 or preceded by a non-alphanumeric character
      if (idx === 0 || !/[a-z0-9]/.test(lower[idx - 1])) {
        if (idx < firstIdx) firstIdx = idx;
        break;
      }
      start = idx + 1;
    }
  }
  return firstIdx === Infinity ? model : model.slice(firstIdx);
}

// ---------------------------------------------------------------------------
// Family rule resolver (generated ordered if-chain)
// ---------------------------------------------------------------------------

function lookupByFamilyRules(provider: string, normalized: string): CapabilityResult | null {
  const lower = normalized.toLowerCase();
  // rule: openai-gpt5-pro, provider: openai, priority: 20
  if (provider === "openai" && (gpt5TokenMatchesGenerated(lower, "gpt-5-pro") || gpt5TokenMatchesGenerated(lower, "gpt5-pro"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["high"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-pro, provider: databricks, priority: 20
  if (provider === "databricks" && (gpt5TokenMatchesGenerated(lower, "gpt-5-pro") || gpt5TokenMatchesGenerated(lower, "gpt5-pro"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["high"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-pro, provider: databricks_v2, priority: 20
  if (provider === "databricks_v2" && (gpt5TokenMatchesGenerated(lower, "gpt-5-pro") || gpt5TokenMatchesGenerated(lower, "gpt5-pro"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["high"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-6, provider: openai, priority: 15
  if (provider === "openai" && (gpt5TokenMatchesGenerated(lower, "gpt-5.6") || gpt5TokenMatchesGenerated(lower, "gpt5.6") || gpt5TokenMatchesGenerated(lower, "gpt-5-6") || gpt5TokenMatchesGenerated(lower, "gpt5-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-6, provider: databricks, priority: 15
  if (provider === "databricks" && (gpt5TokenMatchesGenerated(lower, "gpt-5.6") || gpt5TokenMatchesGenerated(lower, "gpt5.6") || gpt5TokenMatchesGenerated(lower, "gpt-5-6") || gpt5TokenMatchesGenerated(lower, "gpt5-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-6, provider: databricks_v2, priority: 15
  if (provider === "databricks_v2" && (gpt5TokenMatchesGenerated(lower, "gpt-5.6") || gpt5TokenMatchesGenerated(lower, "gpt5.6") || gpt5TokenMatchesGenerated(lower, "gpt-5-6") || gpt5TokenMatchesGenerated(lower, "gpt5-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-5, provider: openai, priority: 15
  if (provider === "openai" && (gpt5TokenMatchesGenerated(lower, "gpt-5.5") || gpt5TokenMatchesGenerated(lower, "gpt5.5") || gpt5TokenMatchesGenerated(lower, "gpt-5-5") || gpt5TokenMatchesGenerated(lower, "gpt5-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-5, provider: databricks, priority: 15
  if (provider === "databricks" && (gpt5TokenMatchesGenerated(lower, "gpt-5.5") || gpt5TokenMatchesGenerated(lower, "gpt5.5") || gpt5TokenMatchesGenerated(lower, "gpt-5-5") || gpt5TokenMatchesGenerated(lower, "gpt5-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-5, provider: databricks_v2, priority: 15
  if (provider === "databricks_v2" && (gpt5TokenMatchesGenerated(lower, "gpt-5.5") || gpt5TokenMatchesGenerated(lower, "gpt5.5") || gpt5TokenMatchesGenerated(lower, "gpt-5-5") || gpt5TokenMatchesGenerated(lower, "gpt5-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-4, provider: openai, priority: 15
  if (provider === "openai" && (gpt5TokenMatchesGenerated(lower, "gpt-5.4") || gpt5TokenMatchesGenerated(lower, "gpt5.4") || gpt5TokenMatchesGenerated(lower, "gpt-5-4") || gpt5TokenMatchesGenerated(lower, "gpt5-4"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-4, provider: databricks, priority: 15
  if (provider === "databricks" && (gpt5TokenMatchesGenerated(lower, "gpt-5.4") || gpt5TokenMatchesGenerated(lower, "gpt5.4") || gpt5TokenMatchesGenerated(lower, "gpt-5-4") || gpt5TokenMatchesGenerated(lower, "gpt5-4"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-4, provider: databricks_v2, priority: 15
  if (provider === "databricks_v2" && (gpt5TokenMatchesGenerated(lower, "gpt-5.4") || gpt5TokenMatchesGenerated(lower, "gpt5.4") || gpt5TokenMatchesGenerated(lower, "gpt-5-4") || gpt5TokenMatchesGenerated(lower, "gpt5-4"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-1, provider: openai, priority: 15
  if (provider === "openai" && (gpt5TokenMatchesGenerated(lower, "gpt-5.1") || gpt5TokenMatchesGenerated(lower, "gpt5.1") || gpt5TokenMatchesGenerated(lower, "gpt-5-1") || gpt5TokenMatchesGenerated(lower, "gpt5-1"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high"] as const,
          defaultEffort: "none",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-1, provider: databricks, priority: 15
  if (provider === "databricks" && (gpt5TokenMatchesGenerated(lower, "gpt-5.1") || gpt5TokenMatchesGenerated(lower, "gpt5.1") || gpt5TokenMatchesGenerated(lower, "gpt-5-1") || gpt5TokenMatchesGenerated(lower, "gpt5-1"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high"] as const,
          defaultEffort: "none",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-1, provider: databricks_v2, priority: 15
  if (provider === "databricks_v2" && (gpt5TokenMatchesGenerated(lower, "gpt-5.1") || gpt5TokenMatchesGenerated(lower, "gpt5.1") || gpt5TokenMatchesGenerated(lower, "gpt-5-1") || gpt5TokenMatchesGenerated(lower, "gpt5-1"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "low", "medium", "high"] as const,
          defaultEffort: "none",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: anthropic-manual-budget-claude3, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-3"))) {
    return {
          registryLabel: null,
          thinkingMode: "manual-budget",
          supportedEfforts: ["low", "medium", "high"] as const,
          defaultEffort: null,
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-manual-budget-claude3, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-3"))) {
    return {
          registryLabel: null,
          thinkingMode: "manual-budget",
          supportedEfforts: ["low", "medium", "high"] as const,
          defaultEffort: null,
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-manual-budget-opus-4-5, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower === "claude-opus-4-5")) {
    return {
          registryLabel: null,
          thinkingMode: "manual-budget",
          supportedEfforts: ["low", "medium", "high"] as const,
          defaultEffort: null,
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-manual-budget-opus-4-5, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower === "claude-opus-4-5")) {
    return {
          registryLabel: null,
          thinkingMode: "manual-budget",
          supportedEfforts: ["low", "medium", "high"] as const,
          defaultEffort: null,
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-opus-4-7, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-opus-4-7"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-opus-4-7, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-opus-4-7"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-opus-4-8, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-opus-4-8"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-opus-4-8, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-opus-4-8"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-opus-5, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-opus-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-opus-5, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-opus-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-sonnet-5, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-sonnet-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-sonnet-5, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-sonnet-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-fable-5, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-fable-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-fable-5, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-fable-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-mythos-5, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-mythos-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-xhigh-mythos-5, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-mythos-5"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-no-xhigh-opus-4-6, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-opus-4-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-no-xhigh-opus-4-6, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-opus-4-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-no-xhigh-sonnet-4-6, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-sonnet-4-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-no-xhigh-sonnet-4-6, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-sonnet-4-6"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-no-xhigh-mythos-preview, provider: anthropic, priority: 10
  if (provider === "anthropic" && (lower.startsWith("claude-mythos-preview"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "none",
        };
  }
  // rule: anthropic-adaptive-no-xhigh-mythos-preview, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (lower.startsWith("claude-mythos-preview"))) {
    return {
          registryLabel: null,
          thinkingMode: "adaptive",
          supportedEfforts: ["low", "medium", "high", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: openai-gpt5-base, provider: openai, priority: 10
  if (provider === "openai" && (gpt5BaseMatchesGenerated(lower, "gpt-5") || gpt5BaseMatchesGenerated(lower, "gpt5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["minimal", "low", "medium", "high"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-base, provider: databricks, priority: 10
  if (provider === "databricks" && (gpt5BaseMatchesGenerated(lower, "gpt-5") || gpt5BaseMatchesGenerated(lower, "gpt5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["minimal", "low", "medium", "high"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "not-applicable",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: openai-gpt5-base, provider: databricks_v2, priority: 10
  if (provider === "databricks_v2" && (gpt5BaseMatchesGenerated(lower, "gpt-5") || gpt5BaseMatchesGenerated(lower, "gpt5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["minimal", "low", "medium", "high"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-standard",
        };
  }
  // rule: dbv2-gpt-code-names-segment, provider: databricks_v2, priority: 6
  if (provider === "databricks_v2" && (gptVersionSegmentMatchesGenerated(lower, "gpt") || gptVersionSegmentMatchesGenerated(lower, "gpt5"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        };
  }
  // rule: dbv2-claude-code-names-segment, provider: databricks_v2, priority: 5
  if (provider === "databricks_v2" && (lower.split(/[^a-z0-9]+/).includes("claude") || lower.split(/[^a-z0-9]+/).includes("opus") || lower.split(/[^a-z0-9]+/).includes("sonnet") || lower.split(/[^a-z0-9]+/).includes("haiku") || lower.split(/[^a-z0-9]+/).includes("mythos") || lower.split(/[^a-z0-9]+/).includes("fable"))) {
    return {
          registryLabel: null,
          thinkingMode: "omit-fields",
          supportedEfforts: ["low", "medium", "high", "xhigh", "max"] as const,
          defaultEffort: "high",
          databricksV2WireRoute: "anthropic-messages",
          normalizationPolicy: "none",
        };
  }
  // rule: dbv2-sol-luna-terra-segment, provider: databricks_v2, priority: 5
  if (provider === "databricks_v2" && (lower.split(/[^a-z0-9]+/).includes("sol") || lower.split(/[^a-z0-9]+/).includes("luna") || lower.split(/[^a-z0-9]+/).includes("terra"))) {
    return {
          registryLabel: null,
          thinkingMode: "none",
          supportedEfforts: ["none", "minimal", "low", "medium", "high", "xhigh"] as const,
          defaultEffort: "medium",
          databricksV2WireRoute: "openai-responses",
          normalizationPolicy: "openai-clamp-max-to-xhigh",
        };
  }
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
  // Step 1: raw exact lookup (case-insensitive — keys lowercased at build time)
  const exactKey = `${provider.toLowerCase()}::${rawModelId.toLowerCase()}`;
  const exact = EXACT_RECORDS.get(exactKey);
  if (exact) return exact;

  // Step 2: family rules on normalized alias
  const normalized = stripCatalogPrefix(rawModelId);
  const family = lookupByFamilyRules(provider, normalized);
  if (family) return family;

  // Step 3: provider fallback
  const isBlank = rawModelId.trim() === "";
  const fb = PROVIDER_FALLBACKS.get(provider) ?? DEFAULT_FALLBACK;
  return isBlank ? { ...fb.blank, registryLabel: null } : { ...fb.concreteUnknown, registryLabel: null };
}
