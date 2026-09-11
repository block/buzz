import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const NOW = new Date("2025-06-15T12:00:00Z").toISOString();

const MANAGED_AGENTS = [
  {
    pubkey: AGENT_PUBKEY,
    name: "Observer Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
];

async function openFeed(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
  );
  await page.getByTestId("channel-agents").click();
  const row = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) })
    .first();
  await row.getByRole("button").first().click();
  await page.getByTestId(`user-profile-view-activity-${AGENT_PUBKEY}`).click();
  return page.getByTestId("agent-session-thread-panel");
}

function event(
  seq: number,
  toolCallId: string,
  update: Record<string, unknown>,
) {
  return {
    seq,
    timestamp: NOW,
    kind: "acp_read",
    agentIndex: 0,
    channelId: CHANNEL_ID,
    sessionId: "session-quality",
    turnId: "turn-quality",
    payload: {
      method: "session/update",
      params: {
        sessionId: "session-quality",
        update: { toolCallId, ...update },
      },
    },
  };
}

async function seed(
  page: import("@playwright/test").Page,
  events: ReturnType<typeof event>[],
) {
  await page.evaluate(
    ({ pubkey, events }) =>
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events,
      }),
    { pubkey: AGENT_PUBKEY, events },
  );
}

test("tool-run group stays readable through live completion and user expansion", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
  const panel = await openFeed(page);

  await seed(page, [
    event(1, "read-a", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/a.ts" },
    }),
    event(2, "read-b", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/b.ts" },
    }),
    event(3, "read-c", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/c.ts" },
    }),
  ]);

  const group = panel.getByTestId("transcript-same-kind-summary");
  await expect(group).toHaveAttribute("open", "");
  await expect(
    group.getByTestId("tool-run-group-status-running"),
  ).toBeVisible();

  await group.locator("summary").first().click();
  await expect(group).not.toHaveAttribute("open", "");
  await group.locator("summary").first().click();
  await expect(group).toHaveAttribute("open", "");

  await seed(page, [
    event(4, "read-a", {
      sessionUpdate: "tool_call_update",
      status: "completed",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/a.ts" },
      rawOutput: "a",
    }),
    event(5, "read-b", {
      sessionUpdate: "tool_call_update",
      status: "completed",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/b.ts" },
      rawOutput: "b",
    }),
    event(6, "read-c", {
      sessionUpdate: "tool_call_update",
      status: "completed",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/c.ts" },
      rawOutput: "c",
    }),
  ]);

  await expect(group).toHaveAttribute("open", "");
  const child = group
    .getByTestId("transcript-tool-item")
    .first()
    .locator("details");
  await child.locator("summary").click();
  await expect(child).toHaveAttribute("open", "");
  await group.locator("summary").first().click();
  await group.locator("summary").first().click();
  await expect(child).toHaveAttribute("open", "");
});

test("an untouched group auto-collapses once its work completes", async ({
  page,
}) => {
  // The counterpart to the stickiness test above: a reader who never touched a
  // group gets it collapsed when the work settles, so a long transcript keeps
  // reading as a narrative instead of a wall of finished steps.
  //
  // This is browser-level because the policy was silently dead in the real app
  // while passing at the unit level. A `<details>` fires `toggle` when React
  // sets `open` on mount, and that echo was recorded as a reader action —
  // latching the sticky `userInteracted` flag on every group that mounted open
  // and disabling automatic collapse forever. Only a rendered DOM shows it.
  await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
  const panel = await openFeed(page);

  await seed(page, [
    event(1, "read-a", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/a.ts" },
    }),
    event(2, "read-b", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/b.ts" },
    }),
  ]);

  // Live work mounts open, and is left strictly untouched.
  const group = panel.getByTestId("transcript-same-kind-summary");
  await expect(group).toHaveAttribute("open", "");

  await seed(page, [
    event(3, "read-a", {
      sessionUpdate: "tool_call_update",
      status: "completed",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/a.ts" },
      rawOutput: "a",
    }),
    event(4, "read-b", {
      sessionUpdate: "tool_call_update",
      status: "completed",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/b.ts" },
      rawOutput: "b",
    }),
  ]);

  // Finished and never touched, so the policy collapses it.
  await expect(group).not.toHaveAttribute("open", "");
  await expect(group.getByTestId("tool-run-group-status-running")).toHaveCount(
    0,
  );
});

test("a collapsed group stays collapsed when its leading call fails and is ejected", async ({
  page,
}) => {
  // Finding B, through the real app: grouping ejects a failed call, so when the
  // FIRST call fails the group's leading leaf changes. A group identity derived
  // from "first leaf" churned there, the store found nothing under the new key,
  // and the card remounted open — silently undoing the reader's collapse.
  await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
  const panel = await openFeed(page);

  // Four reads, so three still remain (and still group) after one is ejected.
  await seed(page, [
    event(1, "read-a", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/a.ts" },
    }),
    event(2, "read-b", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/b.ts" },
    }),
    event(3, "read-c", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/c.ts" },
    }),
    event(4, "read-d", {
      sessionUpdate: "tool_call",
      status: "executing",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/d.ts" },
    }),
  ]);

  // Group membership is asserted by counting child rows rather than reading the
  // "Read N files" label, whose digits are animated per-character.
  const group = panel.getByTestId("transcript-same-kind-summary");
  const groupedSteps = group.getByTestId("transcript-tool-item");
  await expect(groupedSteps).toHaveCount(4);
  await expect(group).toHaveAttribute("open", "");

  // The reader deliberately collapses the running group.
  await group.locator("summary").first().click();
  await expect(group).not.toHaveAttribute("open", "");

  // The LEADING call then fails, which ejects it from the group.
  await seed(page, [
    event(5, "read-a", {
      sessionUpdate: "tool_call_update",
      status: "failed",
      title: "read_file",
      kind: "read_file",
      rawInput: { path: "src/a.ts" },
      rawOutput: "permission denied",
    }),
  ]);

  // The failed call left the group — it is down to the three that still
  // succeeded — and it now stands as its own row in the transcript.
  await expect(groupedSteps).toHaveCount(3);
  await expect(groupedSteps.filter({ hasText: "src/a.ts" })).toHaveCount(0);
  await expect(
    panel.getByTestId("transcript-tool-item").filter({ hasText: "src/a.ts" }),
  ).toHaveCount(1);

  // And the group the reader collapsed is still collapsed — asserted as a
  // state that has to HOLD, not merely be true for one sampled frame. A plain
  // `not.toHaveAttribute` passes the instant it observes "closed", so it would
  // also pass against a card that reopens a beat later; regrouping and the
  // store write both land after the membership change above.
  await expect(group).not.toHaveAttribute("open", "");
  await page.waitForTimeout(500);
  await expect(group).not.toHaveAttribute("open", "");
});
