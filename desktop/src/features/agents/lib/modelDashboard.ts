import type {
  AgentUsageSeries,
  CostField,
  UsageField,
} from "@/shared/api/tauriArchive";

// ── Model origin classification ──────────────────────────────────────────────

/**
 * `frontier` — a closed-weight model served by its lab's API.
 * `open` — an open-weight model, whether served locally or by a host.
 * `unknown` — neither the id nor the catalog says which.
 */
export type ModelOrigin = "frontier" | "open" | "unknown";

export type ModelClassification = {
  origin: ModelOrigin;
  /** True when the harness or model id points at a local runtime. */
  isLocal: boolean;
};

const LOCAL_RUNTIME_MARKERS = [
  "lmstudio",
  "lm-studio",
  "ollama",
  "llama.cpp",
  "llamacpp",
  "llama-server",
  "mlx",
  "gguf",
  "vllm",
  "localhost",
  "127.0.0.1",
];

// Checked before FRONTIER_FAMILY_PREFIXES so `gpt-oss` is not read as `gpt-`.
const OPEN_FAMILY_PREFIXES = [
  "gpt-oss",
  "qwen",
  "qwq",
  "llama",
  "gemma",
  "mistral",
  "mixtral",
  "ministral",
  "devstral",
  "magistral",
  "codestral",
  "deepseek",
  "kimi",
  "glm",
  "minimax",
  "phi-",
  "nemotron",
  "olmo",
  "granite",
  "hermes",
];

const FRONTIER_FAMILY_PREFIXES = [
  "claude",
  "gpt-",
  "chatgpt",
  "o1",
  "o3",
  "o4",
  "gemini",
  "grok",
];

/**
 * Lower-cased id with any `provider/` or `catalog.` routing prefix removed,
 * e.g. `anthropic/claude-opus-5` → `claude-opus-5`.
 */
export function normalizeModelId(model: string): string {
  const lower = model.trim().toLowerCase();
  const slash = lower.lastIndexOf("/");
  return slash >= 0 ? lower.slice(slash + 1) : lower;
}

/**
 * Classify a reported `(harness, model)` pair. `catalog` (from the landscape)
 * settles ids the family prefixes miss; it is optional so usage renders before
 * the catalog loads.
 */
export function classifyModel(
  harness: string | null,
  model: string | null,
  catalog?: ReadonlyMap<string, LandscapeModel>,
): ModelClassification {
  const haystack = `${harness ?? ""} ${model ?? ""}`.toLowerCase();
  // Ollama tags look like `qwen3:30b`; API ids never carry a `:` tag.
  const isLocal =
    LOCAL_RUNTIME_MARKERS.some((marker) => haystack.includes(marker)) ||
    /^[a-z0-9.-]+:[a-z0-9.-]+$/.test(model?.trim().toLowerCase() ?? "");

  if (!model) {
    return { origin: isLocal ? "open" : "unknown", isLocal };
  }

  const id = normalizeModelId(model);
  if (OPEN_FAMILY_PREFIXES.some((prefix) => id.startsWith(prefix))) {
    return { origin: "open", isLocal };
  }

  const entry = catalog?.get(id);
  if (entry) {
    return { origin: entry.openWeights ? "open" : "frontier", isLocal };
  }

  if (FRONTIER_FAMILY_PREFIXES.some((prefix) => id.startsWith(prefix))) {
    return { origin: isLocal ? "open" : "frontier", isLocal };
  }

  return { origin: isLocal ? "open" : "unknown", isLocal };
}

// ── Usage aggregation ────────────────────────────────────────────────────────

export type ModelUsageRow = {
  key: string;
  model: string | null;
  harnesses: string[];
  classification: ModelClassification;
  /** `null` when no turn for this model reported the count. */
  inputTokens: bigint | null;
  outputTokens: bigint | null;
  totalTokens: bigint | null;
  costUsd: number | null;
  /** True when at least one turn left a counter unreported. */
  incomplete: boolean;
  reportCount: number;
  agentCount: number;
};

export type OriginTotals = {
  origin: ModelOrigin;
  totalTokens: bigint;
  costUsd: number;
  reportCount: number;
  modelCount: number;
};

/** Parse a wire token count without `Number(...)` precision loss. */
export function parseTokenCount(value: string | null): bigint | null {
  if (value === null || !/^\d+$/.test(value)) return null;
  return BigInt(value);
}

function addTokens(acc: bigint | null, field: UsageField): bigint | null {
  const value = parseTokenCount(field.value);
  if (value === null) return acc;
  return (acc ?? 0n) + value;
}

function addCost(acc: number | null, field: CostField): number | null {
  if (field.value === null || !Number.isFinite(field.value)) return acc;
  return (acc ?? 0) + field.value;
}

/**
 * Collapse the per-agent model breakdown into one row per model across all
 * agents, sorted by total tokens (largest first, unknown last).
 */
export function aggregateUsageByModel(
  series: AgentUsageSeries,
  catalog?: ReadonlyMap<string, LandscapeModel>,
): ModelUsageRow[] {
  const rows = new Map<
    string,
    ModelUsageRow & { harnessSet: Set<string>; agentSet: Set<string> }
  >();

  for (const agent of series.agents) {
    for (const entry of agent.models) {
      const key = entry.model ? normalizeModelId(entry.model) : "(unreported)";
      let row = rows.get(key);
      if (!row) {
        row = {
          key,
          model: entry.model,
          harnesses: [],
          classification: classifyModel(entry.harness, entry.model, catalog),
          inputTokens: null,
          outputTokens: null,
          totalTokens: null,
          costUsd: null,
          incomplete: false,
          reportCount: 0,
          agentCount: 0,
          harnessSet: new Set(),
          agentSet: new Set(),
        };
        rows.set(key, row);
      }
      if (entry.harness) row.harnessSet.add(entry.harness);
      row.agentSet.add(agent.agentPubkey);
      // Any agent running this model locally makes the row local.
      if (!row.classification.isLocal) {
        const next = classifyModel(entry.harness, entry.model, catalog);
        if (next.isLocal) row.classification = next;
      }
      row.inputTokens = addTokens(row.inputTokens, entry.usage.inputTokens);
      row.outputTokens = addTokens(row.outputTokens, entry.usage.outputTokens);
      row.totalTokens = addTokens(row.totalTokens, entry.usage.totalTokens);
      row.costUsd = addCost(row.costUsd, entry.usage.estimatedCostUsd);
      row.incomplete ||=
        entry.hasUnknownUsage ||
        entry.usage.totalTokens.incomplete ||
        entry.usage.estimatedCostUsd.incomplete;
      row.reportCount += entry.reportCount;
    }
  }

  return [...rows.values()]
    .map(({ harnessSet, agentSet, ...row }) => ({
      ...row,
      harnesses: [...harnessSet].sort(),
      agentCount: agentSet.size,
    }))
    .sort((a, b) => {
      const at = displayTokens(a);
      const bt = displayTokens(b);
      if (at === bt) return b.reportCount - a.reportCount;
      if (at === null) return 1;
      if (bt === null) return -1;
      return at > bt ? -1 : 1;
    });
}

/**
 * Tokens shown for a row: the provider total when reported, otherwise the sum
 * of input and output (some harnesses only report the two halves).
 */
export function displayTokens(row: ModelUsageRow): bigint | null {
  if (row.totalTokens !== null) return row.totalTokens;
  if (row.inputTokens === null && row.outputTokens === null) return null;
  return (row.inputTokens ?? 0n) + (row.outputTokens ?? 0n);
}

export function totalsByOrigin(rows: readonly ModelUsageRow[]): OriginTotals[] {
  const origins: ModelOrigin[] = ["frontier", "open", "unknown"];
  return origins.map((origin) => {
    const matching = rows.filter((row) => row.classification.origin === origin);
    return {
      origin,
      totalTokens: matching.reduce(
        (sum, row) => sum + (displayTokens(row) ?? 0n),
        0n,
      ),
      costUsd: matching.reduce((sum, row) => sum + (row.costUsd ?? 0), 0),
      reportCount: matching.reduce((sum, row) => sum + row.reportCount, 0),
      modelCount: matching.length,
    };
  });
}

// ── Landscape catalog (models.dev) ───────────────────────────────────────────

export const MODELS_DEV_URL = "https://models.dev/api.json";

/**
 * models.dev provider keys for the labs that publish each model first-party.
 * Resellers and routers are left out so every model appears once, with its
 * lab's list price.
 */
export const LANDSCAPE_LABS: Readonly<Record<string, string>> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  google: "Google",
  xai: "xAI",
  meta: "Meta",
  deepseek: "DeepSeek",
  moonshotai: "Moonshot AI",
  alibaba: "Alibaba",
  zai: "Z.ai",
  mistral: "Mistral",
  minimax: "MiniMax",
  nvidia: "NVIDIA",
};

export type LandscapeModel = {
  id: string;
  name: string;
  lab: string;
  openWeights: boolean;
  reasoning: boolean;
  toolCall: boolean;
  contextTokens: number | null;
  /** USD per million tokens; `null` when the lab lists no price. */
  inputCostPerM: number | null;
  outputCostPerM: number | null;
  releaseDate: string | null;
};

function finiteOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * Parse the models.dev payload into landscape rows for {@link LANDSCAPE_LABS},
 * newest release first. Malformed entries are skipped rather than failing the
 * whole catalog.
 */
export function parseModelsDevCatalog(payload: unknown): LandscapeModel[] {
  const root = asRecord(payload);
  if (!root) return [];

  const models: LandscapeModel[] = [];
  for (const [providerKey, lab] of Object.entries(LANDSCAPE_LABS)) {
    const provider = asRecord(root[providerKey]);
    const entries = asRecord(provider?.models);
    if (!entries) continue;

    for (const raw of Object.values(entries)) {
      const model = asRecord(raw);
      if (!model || typeof model.id !== "string") continue;
      const cost = asRecord(model.cost);
      const limit = asRecord(model.limit);
      models.push({
        id: model.id,
        name: typeof model.name === "string" ? model.name : model.id,
        lab,
        openWeights: model.open_weights === true,
        reasoning: model.reasoning === true,
        toolCall: model.tool_call === true,
        contextTokens: finiteOrNull(limit?.context),
        inputCostPerM: finiteOrNull(cost?.input),
        outputCostPerM: finiteOrNull(cost?.output),
        releaseDate:
          typeof model.release_date === "string" ? model.release_date : null,
      });
    }
  }

  return models.sort((a, b) =>
    (b.releaseDate ?? "").localeCompare(a.releaseDate ?? ""),
  );
}

/** Lookup by normalized id, for {@link classifyModel}. First (newest) wins. */
export function indexCatalog(
  models: readonly LandscapeModel[],
): Map<string, LandscapeModel> {
  const index = new Map<string, LandscapeModel>();
  for (const model of models) {
    const key = normalizeModelId(model.id);
    if (!index.has(key)) index.set(key, model);
  }
  return index;
}

export type LandscapeFilter = "all" | "frontier" | "open" | "in-use";

/**
 * Filter and search the catalog. `inUse` is the set of normalized ids the
 * user's agents reported in the current usage window.
 */
export function filterLandscape(
  models: readonly LandscapeModel[],
  {
    filter,
    query,
    inUse,
  }: { filter: LandscapeFilter; query: string; inUse: ReadonlySet<string> },
): LandscapeModel[] {
  const needle = query.trim().toLowerCase();
  return models.filter((model) => {
    if (filter === "frontier" && model.openWeights) return false;
    if (filter === "open" && !model.openWeights) return false;
    if (filter === "in-use" && !inUse.has(normalizeModelId(model.id))) {
      return false;
    }
    if (!needle) return true;
    return (
      model.name.toLowerCase().includes(needle) ||
      model.id.toLowerCase().includes(needle) ||
      model.lab.toLowerCase().includes(needle)
    );
  });
}

// ── Formatting & windows ─────────────────────────────────────────────────────

export function formatTokensCompact(value: bigint): string {
  const units: Array<[bigint, string]> = [
    [1_000_000_000n, "B"],
    [1_000_000n, "M"],
    [1_000n, "K"],
  ];
  for (const [threshold, suffix] of units) {
    if (value >= threshold) {
      return `${Number((value * 10n) / threshold) / 10}${suffix}`;
    }
  }
  return value.toString();
}

export function formatUsd(value: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: value > 0 && value < 1 ? 4 : 2,
  }).format(value);
}

export function formatContext(tokens: number | null): string {
  if (tokens === null) return "—";
  if (tokens >= 1_000_000) return `${Number((tokens / 1_000_000).toFixed(1))}M`;
  return `${Math.round(tokens / 1_000)}K`;
}

export type UsageWindowDays = 7 | 30;

/**
 * Local-midnight Unix-second boundaries covering today plus the preceding
 * `days - 1` days: `days + 1` entries, as `get_agent_usage_series` requires.
 * Each boundary is built with `setDate`, so DST days stay one civil day long.
 */
export function buildLocalDayBoundaries(
  days: UsageWindowDays,
  referenceNow: Date = new Date(),
): number[] {
  const boundaries: number[] = [];
  for (let offset = days - 1; offset >= -1; offset--) {
    const midnight = new Date(referenceNow);
    midnight.setHours(0, 0, 0, 0);
    midnight.setDate(midnight.getDate() - offset);
    boundaries.push(Math.floor(midnight.getTime() / 1_000));
  }
  return boundaries;
}
