import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.use({ video: "on" });

const CHANNEL_ID = "7b4d3d2e-3c15-4e88-9a11-1c0f7d2b6e55";
const OTHER_CHANNEL_ID = "da7ab52e-a378-4888-8291-1a11d0ab8068";
const ROOT_EVENT_ID = "c3".repeat(32);
const TARGET_EVENT_ID = "d4".repeat(32);
const AGENT_PUBKEY = "6".repeat(64);
const MESSAGE_LINK = `buzz://message?channel=${CHANNEL_ID}&id=${TARGET_EVENT_ID}&thread=${ROOT_EVENT_ID}`;
const CANVAS = `# Board context references demo

\`\`\`buzz-workstream-card
${JSON.stringify({
  version: 1,
  synopsis: "Discuss immutable Board context with humans and orchestrators.",
  orchestrator: { pubkey: AGENT_PUBKEY, name: "Board Orchestrator" },
  assignees: [{ pubkey: "7".repeat(64), name: "Field Agent" }],
  pullRequests: ["https://github.com/block/buzz/pull/6357"],
  waitingOn: [
    {
      key: "decision-thread",
      actor: { kind: "person", name: "Logan", pubkey: "8".repeat(64) },
      reason: "Decide the rollout in the cited thread",
      since: "2026-08-19T21:00:00.000Z",
      message: MESSAGE_LINK,
    },
  ],
})}
\`\`\``;

type BoardReference = {
  kind: "workstream" | "thread-blocker";
  identity: string;
  snapshot: { label: string; detail?: string };
  placement: {
    workstreamId: string;
    workstreamLabel: string;
    sectionId: string;
    sectionLabel: string;
  };
  destination?: {
    channelId: string;
    messageId: string;
    threadRootId?: string;
  };
};

async function invoke(
  page: Page,
  command: string,
  payload: Record<string, unknown>,
) {
  return page.evaluate(
    async ({ command: target, payload: args }) => {
      const fn = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!fn) throw new Error("Mock invoke bridge is unavailable.");
      return fn(target, args);
    },
    { command, payload },
  );
}

test("human and orchestrator references replay live and removed Board context", async ({
  page,
}, testInfo) => {
  test.setTimeout(90_000);
  await installMockBridge(page, {
    workstreamBoardFixture: {
      channelId: CHANNEL_ID,
      channelName: "loganj-ws-board-context",
      canvas: CANVAS,
      rootEventId: ROOT_EVENT_ID,
      targetEventId: TARGET_EVENT_ID,
    },
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "Board Orchestrator",
        channelIds: [CHANNEL_ID],
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.getByTestId("open-workstream-board-view").click();
  const board = page.getByTestId("workstream-board-view");
  await expect(board).toBeVisible();
  await board.getByRole("button", { name: "Discuss Board" }).click();

  const options = page.getByTestId(`board-reference-options-${CHANNEL_ID}`);
  await expect(options).toBeVisible();
  await options.getByRole("button", { name: /workstream:/ }).click();
  await options.getByRole("button", { name: /thread-blocker:/ }).click();
  await expect(page.getByText("Reference tray (2)")).toBeVisible();
  await page
    .getByLabel("Discussion message")
    .fill("Human: use this workstream and blocker context.");
  await page.getByRole("button", { name: "Send", exact: true }).click();

  await options.getByRole("button", { name: "Open", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/channels/${CHANNEL_ID}`));
  const humanMessage = page.getByText(
    "Human: use this workstream and blocker context.",
  );
  await expect(humanMessage).toBeVisible();
  await humanMessage
    .locator("xpath=ancestor::*[@data-testid='message-row']")
    .getByTestId("message-board-references")
    .click();
  await expect(page).toHaveURL(/\/workstreams$/);
  await expect(page.getByTestId("board-discussion")).toContainText(
    "Replay 1/2: 2 live",
  );
  const card = page.getByTestId(`workstream-card-${CHANNEL_ID}`);
  await expect(card).toHaveAttribute("aria-current", "true");
  await expect(card).toHaveClass(/border-dashed/);
  await expect(card.getByTestId("current-workstream-reference")).toHaveText(
    "Current reference",
  );
  await expect(
    options.getByRole("button", { name: /workstream:/ }),
  ).toContainText("Referenced");
  await page.getByRole("button", { name: "Next reference" }).click();
  const currentBlocker = options.getByRole("button", {
    name: /thread-blocker:/,
  });
  await expect(currentBlocker).toHaveAttribute("aria-current", "true");
  await expect(currentBlocker).toContainText("Current");
  await page.keyboard.press("ArrowLeft");
  await expect(card).toHaveAttribute("aria-current", "true");
  await page.screenshot({
    path: testInfo.outputPath("human-live-replay.png"),
    fullPage: true,
  });
  await page.waitForTimeout(1_500);

  const crossWorkstreamReference: BoardReference = {
    kind: "thread-blocker",
    identity: "cross-workstream-forgery",
    snapshot: { label: "Other workstream blocker" },
    placement: {
      workstreamId: OTHER_CHANNEL_ID,
      workstreamLabel: "other-workstream",
      sectionId: "blockers",
      sectionLabel: "Blockers",
    },
    destination: {
      channelId: OTHER_CHANNEL_ID,
      messageId: TARGET_EVENT_ID,
      threadRootId: ROOT_EVENT_ID,
    },
  };
  await invoke(page, "send_managed_agent_channel_message", {
    agentPubkey: AGENT_PUBKEY,
    channelId: CHANNEL_ID,
    content: "Forged cross-workstream Board history entry",
    marker: "board-context-cross-workstream",
    boardReferences: [crossWorkstreamReference],
  });
  await page.waitForTimeout(500);
  await expect(page.getByTestId("board-discussion")).not.toContainText(
    "Forged cross-workstream Board history entry",
  );

  const orchestratorWorkstreamReference: BoardReference = {
    kind: "workstream",
    identity: CHANNEL_ID,
    snapshot: { label: "loganj-ws-board-context" },
    placement: {
      workstreamId: CHANNEL_ID,
      workstreamLabel: "loganj-ws-board-context",
      sectionId: "workstream",
      sectionLabel: "Workstream",
    },
  };
  const removedReference: BoardReference = {
    kind: "thread-blocker",
    identity: "removed-deployment-gate",
    snapshot: {
      label: "Removed deployment approval",
      detail: "Release captain",
    },
    placement: {
      workstreamId: CHANNEL_ID,
      workstreamLabel: "loganj-ws-board-context",
      sectionId: "blockers",
      sectionLabel: "Blockers",
    },
    destination: {
      channelId: CHANNEL_ID,
      messageId: TARGET_EVENT_ID,
      threadRootId: ROOT_EVENT_ID,
    },
  };
  await invoke(page, "send_managed_agent_channel_message", {
    agentPubkey: AGENT_PUBKEY,
    channelId: CHANNEL_ID,
    content:
      "Orchestrator: this points to the approval removed after state advanced.",
    marker: "board-context-orchestrator",
    boardReferences: [orchestratorWorkstreamReference, removedReference],
  });

  // Open the workstream conversation, where the orchestrator-authored live
  // event renders its machine-readable reference chip. Clicking the chip
  // carries immutable replay state back to the Board.
  await options.getByRole("button", { name: "Open", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/channels/${CHANNEL_ID}`));
  const orchestratorRow = page.getByText(
    "Orchestrator: this points to the approval removed after state advanced.",
  );
  await expect(orchestratorRow).toBeVisible();
  const orchestratorMessage = orchestratorRow.locator(
    "xpath=ancestor::*[@data-testid='message-row']",
  );
  await orchestratorMessage.getByTestId("message-board-references").click();
  await expect(page).toHaveURL(/\/workstreams$/);
  await expect(page.getByTestId("workstream-board-view")).toBeVisible();
  await expect(page.getByTestId("board-discussion")).toContainText(
    "Replay 1/2: 0 live · 1 changed · 1 historical",
  );
  const historical = page.getByTestId("historical-reference-placeholder");
  await expect(historical).toBeVisible();
  await expect(historical).toContainText(
    "Historical Blockers: Removed deployment approval",
  );
  await expect(historical).not.toHaveAttribute("aria-current");
  await expect(
    page
      .getByTestId(`workstream-card-${CHANNEL_ID}`)
      .getByTestId("current-workstream-reference"),
  ).toHaveText("Current reference");
  await page.screenshot({
    path: testInfo.outputPath("orchestrator-removed-replay.png"),
    fullPage: true,
  });
  await page.waitForTimeout(1_500);

  // Draft text alone is unsent state. Cancelling either exit path preserves it.
  await page.keyboard.press("Escape");
  const draft = page.getByLabel("Discussion message");
  await draft.fill("Keep this unsent draft");
  page.once("dialog", async (dialog) => dialog.dismiss());
  await board.getByRole("button", { name: "Exit discussion" }).click();
  await expect(draft).toHaveValue("Keep this unsent draft");
  page.once("dialog", async (dialog) => dialog.dismiss());
  await page.keyboard.press("Escape");
  await expect(draft).toHaveValue("Keep this unsent draft");
  await page.waitForTimeout(1_500);
});

test("unavailable workstream cards cannot mutate the discussion tray", async ({
  page,
}) => {
  const pageErrors: Error[] = [];
  page.on("pageerror", (error) => pageErrors.push(error));
  await installMockBridge(page, {
    canvasReadError: "Canvas unavailable",
    workstreamBoardFixture: {
      channelId: CHANNEL_ID,
      channelName: "loganj-ws-board-context",
      canvas: CANVAS,
      rootEventId: ROOT_EVENT_ID,
      targetEventId: TARGET_EVENT_ID,
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-workstream-board-view").click();
  const board = page.getByTestId("workstream-board-view");
  await expect(board).toBeVisible();
  const card = page.getByTestId(`workstream-card-${CHANNEL_ID}`);
  await expect(card).toContainText("Card details unavailable");
  await board.getByRole("button", { name: "Discuss Board" }).click();

  await card
    .getByRole("button", { name: /Open #loganj-ws-board-context/ })
    .click();

  await expect(page.getByText("Reference tray (0)")).toBeVisible();
  const unavailableOptions = page.getByTestId(
    `board-reference-options-${CHANNEL_ID}`,
  );
  await expect(unavailableOptions.getByRole("button")).toHaveCount(1);
  await expect(
    unavailableOptions.getByRole("button", { name: "Open", exact: true }),
  ).toBeVisible();
  expect(pageErrors).toEqual([]);
});

test("emptying the tray lets the next workstream become the send destination", async ({
  page,
}) => {
  const channelBId = "4da24e0a-e0b5-4baf-bf3a-9ef26fe88c0d";
  const channelBName = "loganj-ws-board-context-b";
  await installMockBridge(page, {
    workstreamBoardFixtures: [
      {
        channelId: CHANNEL_ID,
        channelName: "loganj-ws-board-context",
        canvas: CANVAS,
        rootEventId: ROOT_EVENT_ID,
        targetEventId: TARGET_EVENT_ID,
      },
      {
        channelId: channelBId,
        channelName: channelBName,
        canvas: CANVAS.replaceAll(CHANNEL_ID, channelBId).replaceAll(
          "loganj-ws-board-context",
          channelBName,
        ),
        rootEventId: "e5".repeat(32),
        targetEventId: "f6".repeat(32),
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-workstream-board-view").click();
  const board = page.getByTestId("workstream-board-view");
  await expect(board).toBeVisible();
  await board.getByRole("button", { name: "Discuss Board" }).click();

  const channelAOptions = page.getByTestId(
    `board-reference-options-${CHANNEL_ID}`,
  );
  await channelAOptions.getByRole("button", { name: /workstream:/ }).click();
  await expect(page.getByText("Reference tray (1)")).toBeVisible();
  await page
    .getByRole("button", { name: "Remove loganj-ws-board-context" })
    .click();
  await expect(page.getByText("Reference tray (0)")).toBeVisible();
  await expect(
    page.getByText(
      "Select an object in one workstream to choose the discussion.",
    ),
  ).toBeVisible();

  const channelBOptions = page.getByTestId(
    `board-reference-options-${channelBId}`,
  );
  await channelBOptions.getByRole("button", { name: /workstream:/ }).click();
  await page
    .getByLabel("Discussion message")
    .fill("Send only through workstream B.");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  const sends = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "send_channel_message",
    ),
  );
  expect(sends).toHaveLength(1);
  expect(sends[0]?.payload).toMatchObject({
    channelId: channelBId,
    boardReferences: [
      {
        placement: { workstreamId: channelBId },
      },
    ],
  });
});
