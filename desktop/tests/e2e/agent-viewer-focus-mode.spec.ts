import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents
const BASE_TIMESTAMP = Date.parse("2025-06-15T12:00:00Z");
const TRANSCRIPT_ROW_COUNT = 40;
/** Newest display block the grouper emits for the seeded transcript. */
const LATEST_ROW_ID = `turn:turn-${TRANSCRIPT_ROW_COUNT - 1}`;

const MANAGED_AGENTS = [
  {
    pubkey: AGENT_PUBKEY,
    name: "Observer Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
];

/**
 * Seeds enough distinct turns to make the transcript taller than its scroll
 * region, so a preserved reading position is observable rather than incidental.
 *
 * One `turnId` per event: the grouper emits a `turn:<turnId>` display block per
 * turn, which is exactly the `data-message-id` the layout switch anchors on.
 */
async function seedLongTranscript(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
  await page.evaluate(
    ({ pubkey, channelId, baseTimestamp, rowCount }) => {
      const events = Array.from({ length: rowCount }, (_unused, index) => ({
        seq: index + 1,
        timestamp: new Date(baseTimestamp + index * 1_000).toISOString(),
        kind: "acp_read",
        agentIndex: 0,
        channelId,
        sessionId: "session-focus",
        turnId: `turn-${index}`,
        payload: {
          method: "session/update",
          params: {
            sessionId: "session-focus",
            update: {
              sessionUpdate: "agent_message_chunk",
              messageId: `message-${index}`,
              content: {
                type: "text",
                text: `Activity row ${index}: a deliberately long line of assistant output so that changing the reading measure forces the transcript to reflow and re-measure every row below it.`,
              },
            },
          },
        },
      }));
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events,
      });
    },
    {
      pubkey: AGENT_PUBKEY,
      channelId: CHANNEL_ID,
      baseTimestamp: BASE_TIMESTAMP,
      rowCount: TRANSCRIPT_ROW_COUNT,
    },
  );
}

async function openAgentsChannel(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
}

/**
 * Opens the viewer through the profile panel's activity ingress.
 *
 * With a seeded transcript the panel renders the live-activity embed rather than
 * the empty "Activity log" row, so the ingress is the embed's open button. The
 * panel may already be open underneath a just-closed viewer (closing the viewer
 * does not close the profile pane), so only open it when it is absent —
 * re-clicking the message row would toggle it shut.
 */
async function openAgentViewer(page: import("@playwright/test").Page) {
  const profilePanel = page.getByTestId("user-profile-panel");
  if ((await profilePanel.count()) === 0) {
    const messageRow = page
      .getByTestId("message-row")
      .filter({ has: page.getByText("Observer Agent", { exact: false }) });
    await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
    await messageRow.first().getByRole("button").first().click();
  }
  await expect(profilePanel).toBeVisible({ timeout: 10_000 });

  const openActivity = profilePanel.getByRole("button", {
    name: /Open full activity/,
  });
  await expect(openActivity).toBeVisible({ timeout: 10_000 });
  await openActivity.click();

  const panel = page.getByTestId("agent-session-thread-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  return panel;
}

/**
 * Scrolls off the tail so a subsequent snap-to-latest is observable rather than
 * incidental.
 */
async function scrollAwayFromBottom(
  body: import("@playwright/test").Locator,
): Promise<void> {
  await expect
    .poll(
      async () =>
        await body.evaluate((element) => {
          const maxScrollTop = element.scrollHeight - element.clientHeight;
          if (maxScrollTop <= 0) return false;

          const targetScrollTop = Math.floor(maxScrollTop * 0.4);
          element.scrollTop = targetScrollTop;
          element.dispatchEvent(new Event("scroll", { bubbles: true }));
          return Math.abs(element.scrollTop - targetScrollTop) <= 1;
        }),
      { timeout: 15_000 },
    )
    .toBe(true);
}

/**
 * Asserts the transcript is showing the newest activity.
 *
 * The viewer deliberately snaps to latest across a layout switch instead of
 * restoring the reader's row: transcript rows use `content-visibility: auto`
 * with an over-reserved `contain-intrinsic-size`, so the scroll height shrinks
 * as rows paint after the reflow and any restored offset slides out from under
 * the reader. Snapping is the behaviour we can actually keep.
 */
async function expectSnappedToLatest(
  page: import("@playwright/test").Page,
  body: import("@playwright/test").Locator,
) {
  await expect(
    page.locator(`[data-message-id="${LATEST_ROW_ID}"]`),
  ).toBeVisible({ timeout: 10_000 });
  await expect
    .poll(
      async () =>
        await body.evaluate(
          (element) =>
            element.scrollHeight - element.clientHeight - element.scrollTop,
        ),
      { timeout: 10_000 },
    )
    .toBeLessThanOrEqual(32);
}

test("the agent work viewer shares the thread's focus drawer and dismissal", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.channels.threadViewMode", "focus");
  });
  await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await seedLongTranscript(page);
  await openAgentsChannel(page);

  const panel = await openAgentViewer(page);
  await expect(page).toHaveURL(new RegExp(`agentSession=${AGENT_PUBKEY}`));

  // The viewer is the drawer's occupant, named for itself rather than "Thread".
  const channel = page.getByTestId("channel-drop-zone");
  const drawer = page.getByTestId("focus-thread-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer).toHaveAttribute("aria-label", "Agent activity");
  await expect(channel).toHaveAttribute("inert", "");
  // The scrim sliver is the way back, so the header arrow must not compete.
  await expect(page.getByTestId("agent-session-back")).toHaveCount(0);
  // Read-only and channel-scoped boundaries are unchanged by the presentation.
  await expect(page.getByTestId("agent-session-scope-label")).toHaveText(
    "Activity · #agents",
  );
  await expect(panel.getByTestId("message-input")).toHaveCount(0);

  const body = page.getByTestId("agent-session-transcript-body");
  const readingColumn = body.locator(":scope > div").nth(1);
  await expect
    .poll(async () => {
      const [drawerBox, columnBox] = await Promise.all([
        drawer.boundingBox(),
        readingColumn.boundingBox(),
      ]);
      if (!drawerBox || !columnBox) return null;
      return {
        centered:
          Math.abs(
            columnBox.x +
              columnBox.width / 2 -
              (drawerBox.x + drawerBox.width / 2),
          ) <= 1,
        withinReadingMeasure: columnBox.width <= 880,
      };
    })
    .toEqual({ centered: true, withinReadingMeasure: true });
  await scrollAwayFromBottom(body);

  // focus → split, driven by the shared toggle in the viewer's header. Its
  // accessible name describes the viewer, not the thread that shares the
  // preference.
  await page
    .getByRole("button", { name: "Show activity beside channel" })
    .click();
  await expect(drawer).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expectSnappedToLatest(page, body);

  // split → focus: the transcript re-pins to latest on the return trip too.
  await scrollAwayFromBottom(body);
  await page.getByRole("button", { name: "Expand activity" }).click();
  await expect(drawer).toBeVisible();
  await expect(channel).toHaveAttribute("inert", "");
  await expectSnappedToLatest(page, body);

  // Escape closes the AGENT SESSION. Were it wired to thread close, the drawer
  // would stay up with the viewer still inside it.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(page.getByTestId("agent-session-thread-panel")).toHaveCount(0);
  await expect(page).not.toHaveURL(/agentSession=/);
  await expect(channel).not.toHaveAttribute("inert", "");

  // Browser history restores the channel-scoped viewer and its persisted focus
  // presentation; closing it again must remove only the viewer URL state.
  await page.goBack();
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expect(drawer).toBeVisible();
  await expect(page.getByTestId("agent-session-scope-label")).toHaveText(
    "Activity · #agents",
  );
  await page.goForward();
  await expect(page.getByTestId("agent-session-thread-panel")).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");

  // Scrim click dismisses the same surface.
  await openAgentViewer(page);
  await expect(drawer).toBeVisible();
  await page
    .getByTestId("focus-thread-drawer-scrim")
    .click({ position: { x: 24, y: 200 } });
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(page.getByTestId("agent-session-thread-panel")).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");

  // Sidebar background dismissal is the overlay presentation's, and it too must
  // reach the viewer rather than a thread.
  await openAgentViewer(page);
  await expect(drawer).toBeVisible();
  await page
    .getByTestId("app-sidebar-scroll-anchor")
    .evaluate((element) => (element as HTMLElement).click());
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(page.getByTestId("agent-session-thread-panel")).toHaveCount(0);
});

test("narrow viewers do not offer an unavailable layout switch", async ({
  page,
}) => {
  await page.setViewportSize({ width: 860, height: 720 });
  await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await seedLongTranscript(page);
  await openAgentsChannel(page);

  await openAgentViewer(page);
  await expect(page.getByTestId("focus-thread-drawer")).toHaveCount(0);
  await expect(page.getByTestId("thread-view-mode-toggle")).toHaveCount(0);
});
