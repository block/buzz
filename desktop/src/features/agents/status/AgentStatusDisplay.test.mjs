import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
  AgentStatusMobileBar,
  AgentStatusSidebarPanel,
} from "./AgentStatusDisplay.tsx";
import { deriveConfiguredAgentStatuses } from "./agentStatusModel.ts";
import { decodeLatestAgentMetricSnapshot } from "./useAgentStatusAdapter.ts";

const NOW_SECONDS = 1_789_000_000;
const VIC_TRA_PUBKEY = "a".repeat(64);
const CLAUDE_PUBKEY = "b".repeat(64);

const agents = [
  { id: "victra", label: "Victra", agentPubkey: VIC_TRA_PUBKEY },
  { id: "claude", label: "Claude", agentPubkey: CLAUDE_PUBKEY },
];

function snapshot(overrides = {}) {
  return {
    agentPubkey: VIC_TRA_PUBKEY,
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
      agentPubkey: VIC_TRA_PUBKEY,
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
      agents: [agents[0], { id: "claude", label: "Claude" }],
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
        agentPubkey: CLAUDE_PUBKEY,
        contextUsedTokens: 10_000n,
        contextLimitTokens: 200_000n,
        model: "sonnet-4",
      }),
    ],
  });

  it("renders context, every provider window, reset time and data age", () => {
    const markup = renderToStaticMarkup(
      AgentStatusSidebarPanel({ statuses, nowSeconds: NOW_SECONDS }),
    );

    assert.match(markup, /Victra/);
    assert.match(markup, /75%/);
    assert.match(markup, /5 hour/);
    assert.match(markup, /38%/);
    assert.match(markup, /Weekly/);
    assert.match(markup, /72%/);
    assert.match(markup, /Resets in 1h/);
    assert.match(markup, /Updated 2m ago/);
    assert.equal((markup.match(/role="progressbar"/g) ?? []).length, 4);
  });

  it("uses permanent desktop and compact fixed mobile placement classes", () => {
    const desktop = renderToStaticMarkup(
      AgentStatusSidebarPanel({ statuses, nowSeconds: NOW_SECONDS }),
    );
    const mobile = renderToStaticMarkup(
      AgentStatusMobileBar({ statuses, nowSeconds: NOW_SECONDS }),
    );

    assert.match(desktop, /class="[^"]*hidden[^"]*md:block/);
    assert.match(mobile, /class="[^"]*fixed[^"]*md:hidden/);
    assert.match(mobile, /aria-label="Agent status"/);
  });

  it("renders honest unavailable states without invented metric values", () => {
    const emptyStatuses = deriveConfiguredAgentStatuses({
      agents,
      nowSeconds: NOW_SECONDS,
      snapshots: [],
    });
    const markup = renderToStaticMarkup(
      AgentStatusMobileBar({
        statuses: emptyStatuses,
        nowSeconds: NOW_SECONDS,
      }),
    );

    assert.match(markup, /Waiting for metrics/);
    assert.doesNotMatch(markup, /0%/);
  });
});
