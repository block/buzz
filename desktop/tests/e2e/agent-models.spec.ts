import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const usage = (counts: {
  input?: string;
  output?: string;
  total?: string;
  cost?: number;
}) => ({
  inputTokens: { value: counts.input ?? null, incomplete: false },
  outputTokens: { value: counts.output ?? null, incomplete: false },
  totalTokens: { value: counts.total ?? null, incomplete: false },
  estimatedCostUsd: { value: counts.cost ?? null, incomplete: false },
  cacheReadTokens: { value: null, incomplete: false },
  cacheWriteTokens: { value: null, incomplete: false },
  freshInputTokens: { value: null, incomplete: false },
});

const model = (
  harness: string,
  id: string,
  counts: Parameters<typeof usage>[0],
  reportCount: number,
) => ({
  harness,
  model: id,
  usage: usage(counts),
  reportCount,
  hasUnknownUsage: false,
});

const AGENT_USAGE_SERIES = {
  collectionEnabled: true,
  buckets: [],
  agents: [
    {
      agentPubkey: "a1".repeat(32),
      usage: usage({}),
      buckets: [],
      models: [
        model(
          "claude-code",
          "claude-opus-5",
          { input: "2400000", output: "310000", total: "2710000", cost: 19.75 },
          142,
        ),
        model(
          "lmstudio",
          "qwen3-coder-30b",
          { input: "5100000", output: "640000" },
          388,
        ),
      ],
      reportCount: 530,
      hasUnknownUsage: false,
    },
    {
      agentPubkey: "b2".repeat(32),
      usage: usage({}),
      buckets: [],
      models: [
        model(
          "codex",
          "gpt-5.5",
          { input: "900000", output: "120000", total: "1020000", cost: 6.1 },
          61,
        ),
        model(
          "goose",
          "openrouter/deepseek-v4",
          { input: "700000", output: "90000", total: "790000", cost: 0.32 },
          40,
        ),
      ],
      reportCount: 101,
      hasUnknownUsage: false,
    },
  ],
  coverage: {
    firstArchivedAt: null,
    lastArchivedAt: null,
    firstReportedAt: null,
    lastReportedAt: null,
    reportCount: 631,
    invalidReportCount: 0,
    hasUnknownUsage: false,
  },
  hasArchivedEvidence: null,
};

const MODELS_DEV = {
  anthropic: {
    models: {
      "claude-opus-5": {
        id: "claude-opus-5",
        name: "Claude Opus 5",
        open_weights: false,
        reasoning: true,
        tool_call: true,
        release_date: "2026-07-24",
        limit: { context: 1_000_000 },
        cost: { input: 5, output: 25 },
      },
    },
  },
  openai: {
    models: {
      "gpt-5.5": {
        id: "gpt-5.5",
        name: "GPT-5.5",
        open_weights: false,
        reasoning: true,
        tool_call: true,
        release_date: "2026-06-11",
        limit: { context: 400_000 },
        cost: { input: 1.25, output: 10 },
      },
    },
  },
  deepseek: {
    models: {
      "deepseek-v4": {
        id: "deepseek-v4",
        name: "DeepSeek V4",
        open_weights: true,
        reasoning: true,
        tool_call: true,
        release_date: "2026-10-17",
        limit: { context: 131_072 },
        cost: { input: 0.3, output: 1.2 },
      },
    },
  },
};

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, { agentUsageSeries: AGENT_USAGE_SERIES });
  await page.route("https://models.dev/api.json", (route) =>
    route.fulfill({ json: MODELS_DEV }),
  );
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
});

test("usage tab splits frontier and local model usage", async ({ page }) => {
  const section = page.getByTestId("agents-models");
  await section.scrollIntoViewIfNeeded();

  await expect(page.getByTestId("agents-models-usage-row")).toHaveCount(4);
  await expect(page.getByTestId("agents-models-total-frontier")).toContainText(
    "3.7M",
  );
  await expect(page.getByTestId("agents-models-total-open")).toContainText(
    "6.5M",
  );
  await expect(
    page.getByTestId("agents-models-usage-row").first(),
  ).toContainText("qwen3-coder-30b");
  await expect(
    page.getByTestId("agents-models-usage-row").first(),
  ).toContainText("Local");

  await waitForAnimations(page);
  await section.screenshot({
    path: "test-results/agent-models/usage.png",
  });
});

test("landscape tab lists catalog models and marks the ones in use", async ({
  page,
}) => {
  const section = page.getByTestId("agents-models");
  await section.scrollIntoViewIfNeeded();
  await page.getByTestId("agents-models-tab-landscape").click();

  const rows = page.getByTestId("agents-models-landscape-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.first()).toContainText("DeepSeek V4");
  await expect(rows.first()).toContainText("In use");

  await page.getByTestId("agents-models-landscape-filter-open").click();
  await expect(rows).toHaveCount(1);

  await page.getByTestId("agents-models-landscape-filter-all").click();
  await page.getByLabel("Search models").fill("anthropic");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("Claude Opus 5");

  await page.getByLabel("Search models").fill("");
  await waitForAnimations(page);
  await section.screenshot({
    path: "test-results/agent-models/landscape.png",
  });
});
