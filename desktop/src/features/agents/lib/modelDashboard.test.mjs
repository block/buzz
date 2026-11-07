import assert from "node:assert/strict";
import test from "node:test";

import {
  aggregateUsageByModel,
  buildLocalDayBoundaries,
  classifyModel,
  displayTokens,
  filterLandscape,
  formatContext,
  formatTokensCompact,
  indexCatalog,
  normalizeModelId,
  parseModelsDevCatalog,
  totalsByOrigin,
} from "./modelDashboard.ts";

const field = (value) => ({ value, incomplete: false });
const costField = (value) => ({ value, incomplete: false });

function usage({ input = null, output = null, total = null, cost = null }) {
  return {
    inputTokens: field(input),
    outputTokens: field(output),
    totalTokens: field(total),
    estimatedCostUsd: costField(cost),
    cacheReadTokens: field(null),
    cacheWriteTokens: field(null),
    freshInputTokens: field(null),
  };
}

function modelEntry(harness, model, counts, reportCount = 1) {
  return {
    harness,
    model,
    usage: usage(counts),
    reportCount,
    hasUnknownUsage: false,
  };
}

function series(agents) {
  return {
    collectionEnabled: true,
    buckets: [],
    agents: agents.map(([agentPubkey, models]) => ({
      agentPubkey,
      usage: usage({}),
      buckets: [],
      models,
      reportCount: models.reduce((sum, m) => sum + m.reportCount, 0),
      hasUnknownUsage: false,
    })),
    coverage: {
      firstArchivedAt: null,
      lastArchivedAt: null,
      firstReportedAt: null,
      lastReportedAt: null,
      reportCount: 0,
      invalidReportCount: 0,
      hasUnknownUsage: false,
    },
    hasArchivedEvidence: null,
  };
}

const MODELS_DEV_FIXTURE = {
  anthropic: {
    models: {
      "claude-opus-5": {
        id: "claude-opus-5",
        name: "Claude Opus 5",
        open_weights: false,
        reasoning: true,
        tool_call: true,
        release_date: "2026-07-24",
        limit: { context: 1_000_000, output: 128_000 },
        cost: { input: 5, output: 25 },
      },
    },
  },
  deepseek: {
    models: {
      "deepseek-v4": {
        id: "deepseek-v4",
        name: "DeepSeek V4",
        open_weights: true,
        release_date: "2026-10-17",
        limit: { context: 131_072 },
        cost: { input: 0.3, output: 1.2 },
      },
      broken: { name: "no id" },
    },
  },
  "some-reseller": {
    models: { "claude-opus-5": { id: "claude-opus-5", name: "Resold" } },
  },
};

test("normalizeModelId strips routing prefixes and case", () => {
  assert.equal(normalizeModelId("Anthropic/Claude-Opus-5"), "claude-opus-5");
  assert.equal(normalizeModelId("qwen3:30b"), "qwen3:30b");
});

test("classifyModel — frontier API ids", () => {
  assert.deepEqual(classifyModel("claude-code", "claude-opus-5"), {
    origin: "frontier",
    isLocal: false,
  });
  assert.equal(classifyModel("codex", "gpt-5.5").origin, "frontier");
  assert.equal(classifyModel(null, "openai/o4-mini").origin, "frontier");
});

test("classifyModel — gpt-oss is open-weight, not frontier", () => {
  assert.equal(classifyModel("goose", "gpt-oss-120b").origin, "open");
});

test("classifyModel — local runtimes and Ollama tags are local", () => {
  assert.deepEqual(classifyModel("buzz-agent", "qwen3:30b"), {
    origin: "open",
    isLocal: true,
  });
  assert.deepEqual(classifyModel("lmstudio", "my-finetune"), {
    origin: "open",
    isLocal: true,
  });
});

test("classifyModel — catalog settles ids the prefixes miss", () => {
  const catalog = indexCatalog(parseModelsDevCatalog(MODELS_DEV_FIXTURE));
  assert.equal(classifyModel(null, "mystery-model").origin, "unknown");
  const custom = new Map(catalog);
  custom.set("mystery-model", { ...catalog.get("claude-opus-5") });
  assert.equal(classifyModel(null, "mystery-model", custom).origin, "frontier");
});

test("aggregateUsageByModel merges a model across agents and sorts by tokens", () => {
  const rows = aggregateUsageByModel(
    series([
      [
        "a".repeat(64),
        [
          modelEntry("claude-code", "claude-opus-5", {
            input: "1000",
            output: "500",
            total: "1500",
            cost: 0.02,
          }),
          modelEntry("lmstudio", "qwen3-coder-30b", {
            input: "90000",
            output: "10000",
          }),
        ],
      ],
      [
        "b".repeat(64),
        [
          modelEntry(
            "claude-code",
            "anthropic/claude-opus-5",
            { input: "2000", output: "1000", total: "3000", cost: 0.04 },
            3,
          ),
        ],
      ],
    ]),
  );

  assert.equal(rows.length, 2);
  assert.equal(rows[0].key, "qwen3-coder-30b");
  assert.equal(displayTokens(rows[0]), 100_000n);
  assert.equal(rows[0].classification.isLocal, true);

  const opus = rows[1];
  assert.equal(opus.totalTokens, 4500n);
  assert.equal(opus.agentCount, 2);
  assert.equal(opus.reportCount, 4);
  assert.ok(Math.abs(opus.costUsd - 0.06) < 1e-9);
  assert.equal(opus.classification.origin, "frontier");
});

test("aggregateUsageByModel keeps unreported counts as null, not zero", () => {
  const [row] = aggregateUsageByModel(
    series([["c".repeat(64), [modelEntry("goose", "glm-5", {})]]]),
  );
  assert.equal(row.totalTokens, null);
  assert.equal(row.costUsd, null);
  assert.equal(displayTokens(row), null);
});

test("totalsByOrigin splits frontier and open-weight spend", () => {
  const rows = aggregateUsageByModel(
    series([
      [
        "d".repeat(64),
        [
          modelEntry("claude-code", "claude-opus-5", { total: "100", cost: 1 }),
          modelEntry("lmstudio", "gemma-4", { total: "900" }),
        ],
      ],
    ]),
  );
  const [frontier, open, unknown] = totalsByOrigin(rows);
  assert.equal(frontier.totalTokens, 100n);
  assert.equal(frontier.costUsd, 1);
  assert.equal(open.totalTokens, 900n);
  assert.equal(open.costUsd, 0);
  assert.equal(unknown.modelCount, 0);
});

test("parseModelsDevCatalog keeps first-party labs only, newest first", () => {
  const models = parseModelsDevCatalog(MODELS_DEV_FIXTURE);
  assert.deepEqual(
    models.map((m) => [m.id, m.lab, m.openWeights]),
    [
      ["deepseek-v4", "DeepSeek", true],
      ["claude-opus-5", "Anthropic", false],
    ],
  );
  assert.equal(models[1].inputCostPerM, 5);
  assert.equal(models[0].reasoning, false);
  assert.deepEqual(parseModelsDevCatalog(null), []);
  assert.deepEqual(parseModelsDevCatalog([1, 2]), []);
});

test("filterLandscape filters by weights, in-use set, and search", () => {
  const models = parseModelsDevCatalog(MODELS_DEV_FIXTURE);
  const inUse = new Set(["claude-opus-5"]);
  const ids = (filter, query = "") =>
    filterLandscape(models, { filter, query, inUse }).map((m) => m.id);
  assert.deepEqual(ids("frontier"), ["claude-opus-5"]);
  assert.deepEqual(ids("open"), ["deepseek-v4"]);
  assert.deepEqual(ids("in-use"), ["claude-opus-5"]);
  assert.deepEqual(ids("all", "deepseek"), ["deepseek-v4"]);
  assert.deepEqual(ids("all", "ANTHROPIC"), ["claude-opus-5"]);
});

test("formatters", () => {
  assert.equal(formatTokensCompact(999n), "999");
  assert.equal(formatTokensCompact(1_234n), "1.2K");
  assert.equal(formatTokensCompact(5_000_000n), "5M");
  assert.equal(formatContext(1_000_000), "1M");
  assert.equal(formatContext(131_072), "131K");
  assert.equal(formatContext(null), "—");
});

test("buildLocalDayBoundaries returns days + 1 ascending local midnights", () => {
  const now = new Date(2026, 9, 29, 15, 30);
  const week = buildLocalDayBoundaries(7, now);
  assert.equal(week.length, 8);
  assert.equal(week[6], Math.floor(new Date(2026, 9, 29).getTime() / 1000));
  assert.equal(week[7], Math.floor(new Date(2026, 9, 30).getTime() / 1000));
  assert.equal(week[0], Math.floor(new Date(2026, 9, 23).getTime() / 1000));
  for (let i = 1; i < week.length; i++) assert.ok(week[i] > week[i - 1]);
  assert.equal(buildLocalDayBoundaries(30, now).length, 31);
});
