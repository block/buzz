import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
  AgentStatusDetails,
  AgentStatusIndicator,
  AgentStatusSidebarPanel,
} from "./AgentStatusDisplay.tsx";
import { deriveConfiguredAgentStatuses } from "./agentStatusModel.ts";
import { decodeLatestAgentMetricSnapshot } from "./useAgentStatusAdapter.ts";

const NOW_SECONDS = 1_789_000_000;
const AGENT_ALPHA_PUBKEY = "a".repeat(64);
const AGENT_BETA_PUBKEY = "b".repeat(64);

const agents = [
  { id: "agent-alpha", label: "Agent Alpha", agentPubkey: AGENT_ALPHA_PUBKEY },
  { id: "agent-beta", label: "Agent Beta", agentPubkey: AGENT_BETA_PUBKEY },
];

function snapshot(overrides = {}) {
  return {
    agentPubkey: AGENT_ALPHA_PUBKEY,
    model: "opus-4.1",
    harness: "claude-code",
    contextUsedTokens: 75_000n,
    contextLimitTokens: 100_000n,
    accountUsageWindows: [],
    timestamp: NOW_SECONDS - 120,
    ...overrides,
  };
}

describe("configured agent status model", () => {
  it("decodes RFC3339 windows and preserves u64 token precision", () => {
    const decoded = decodeLatestAgentMetricSnapshot({
      agentPubkey: AGENT_ALPHA_PUBKEY,
      model: "gpt-5.6-sol",
      harness: "hermes-agent",
      contextUsedTokens: "18446744073709551615",
      contextLimitTokens: "18446744073709551615",
      accountUsageWindows: [
        {
          label: "Session",
          usedPercent: 42,
          resetAt: "2026-09-05T12:00:00Z",
        },
      ],
      timestamp: "2026-09-05T11:00:00Z",
    });

    assert.equal(decoded.contextUsedTokens, 18_446_744_073_709_551_615n);
    assert.equal(decoded.contextLimitTokens, 18_446_744_073_709_551_615n);
    assert.equal(decoded.accountUsageWindows[0].resetAt, 1_788_609_600);
    assert.equal(decoded.timestamp, 1_788_606_000);
  });

  it("matches exact pubkeys and keeps only the newest snapshot", () => {
    const statuses = deriveConfiguredAgentStatuses({
      agents,
      nowSeconds: NOW_SECONDS,
      snapshots: [
        snapshot({ contextUsedTokens: 20_000n, timestamp: NOW_SECONDS - 300 }),
        snapshot(),
        snapshot({ agentPubkey: "c".repeat(64), contextUsedTokens: 99_000n }),
      ],
    });

    assert.equal(statuses[0].state, "ready");
    assert.equal(statuses[0].contextPercent, 75);
    assert.equal(statuses[0].ageSeconds, 120);
    assert.equal(statuses[1].state, "empty");
  });

  it("clamps malformed percentages and marks old snapshots stale", () => {
    const [status] = deriveConfiguredAgentStatuses({
      agents: [agents[0]],
      nowSeconds: NOW_SECONDS,
      snapshots: [
        snapshot({
          contextUsedTokens: 120_000n,
          accountUsageWindows: [
            { label: "5 hour", usedPercent: -4 },
            { label: "Weekly", usedPercent: 140 },
          ],
          timestamp: NOW_SECONDS - 901,
        }),
      ],
      staleAfterSeconds: 900,
    });

    assert.equal(status.state, "stale");
    assert.equal(status.contextPercent, 100);
    assert.deepEqual(
      status.usageWindows.map(({ usedPercent }) => usedPercent),
      [0, 100],
    );
  });

  it("keeps source errors distinct from missing and unconfigured data", () => {
    const statuses = deriveConfiguredAgentStatuses({
      agents: [agents[0], { id: "unconfigured", label: "Unconfigured agent" }],
      error: "Metrics unavailable",
      nowSeconds: NOW_SECONDS,
      snapshots: [],
    });

    assert.equal(statuses[0].state, "error");
    assert.equal(statuses[0].errorMessage, "Metrics unavailable");
    assert.equal(statuses[1].state, "unconfigured");
  });
});

describe("agent status surfaces", () => {
  const statuses = deriveConfiguredAgentStatuses({
    agents,
    nowSeconds: NOW_SECONDS,
    snapshots: [
      snapshot({
        accountUsageWindows: [
          {
            label: "5 hour",
            usedPercent: 38,
            resetAt: NOW_SECONDS + 3_600,
          },
          { label: "Weekly", usedPercent: 72 },
        ],
      }),
      snapshot({
        agentPubkey: AGENT_BETA_PUBKEY,
        contextUsedTokens: 10_000n,
        contextLimitTokens: 200_000n,
        model: "sonnet-4",
      }),
    ],
  });

  it("renders a compact percentage ring and full on-demand details", () => {
    const indicator = renderToStaticMarkup(
      AgentStatusIndicator({ status: statuses[0] }),
    );
    const details = renderToStaticMarkup(
      AgentStatusDetails({ status: statuses[0], nowSeconds: NOW_SECONDS }),
    );

    assert.match(indicator, /aria-label="Agent Alpha usage: 75%"/);
    assert.match(indicator, />75%</);
    assert.doesNotMatch(indicator, /5 hour/);
    assert.match(details, /75K \/ 100K tokens/);
    assert.match(details, /5 hour/);
    assert.match(details, /38%/);
    assert.match(details, /Weekly/);
    assert.match(details, /72%/);
    assert.match(details, /Resets in 1h/);
    assert.match(details, /Updated 2m ago/);
  });

  it("shows the highest known usage in the compact ring", () => {
    const [status] = deriveConfiguredAgentStatuses({
      agents: [agents[0]],
      nowSeconds: NOW_SECONDS,
      snapshots: [
        snapshot({
          accountUsageWindows: [{ label: "Session", usedPercent: 87 }],
        }),
      ],
    });
    const indicator = renderToStaticMarkup(AgentStatusIndicator({ status }));

    assert.match(indicator, /aria-label="Agent Alpha usage: 87%"/);
  });

  it("lists agent names compactly instead of rendering permanent metric cards", () => {
    const desktop = renderToStaticMarkup(
      AgentStatusSidebarPanel({ statuses, nowSeconds: NOW_SECONDS }),
    );

    assert.match(desktop, /aria-label="Agent usage"/);
    assert.match(desktop, /Agent Alpha/);
    assert.match(desktop, /Agent Beta/);
    assert.doesNotMatch(desktop, /role="progressbar"/);
    assert.doesNotMatch(desktop, /Context unavailable/);
  });

  it("renders honest unavailable states without invented metric values", () => {
    const emptyStatuses = deriveConfiguredAgentStatuses({
      agents,
      nowSeconds: NOW_SECONDS,
      snapshots: [],
    });
    const markup = renderToStaticMarkup(
      AgentStatusIndicator({ status: emptyStatuses[0] }),
    );

    assert.match(markup, /aria-label="Agent Alpha usage unavailable"/);
    assert.doesNotMatch(markup, /0%/);
  });
});
