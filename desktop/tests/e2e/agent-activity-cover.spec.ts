import { expect, test, type Page } from "@playwright/test";

import { KIND_TYPING_INDICATOR } from "../../src/shared/constants/kinds";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const AGENT_PUBKEY = TEST_IDENTITIES.alice.pubkey;

/** Two panes fit, so the agent panel covers. */
const WIDE_VIEWPORT = { width: 1280, height: 800 };

/** Below the two-pane breakpoint, so today's presentation is unchanged. */
const NARROW_VIEWPORT = { width: 860, height: 800 };

async function waitForMockLiveSubscription(
  page: Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ currentChannelName, currentKind }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
                kind?: number;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: currentChannelName,
            kind: currentKind,
          }) ?? false,
        { currentChannelName: channelName, currentKind: kind },
      ),
    )
    .toBe(true);
}

async function seedThreadRoot(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  return page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "agents",
      content: "Cover drawer exclusivity thread",
      createdAt: 1_700_800_000,
    });
    if (!root) throw new Error("Failed to seed thread root");
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "agents",
      content: "A reply so the thread summary renders.",
      parentEventId: root.id,
      createdAt: 1_700_800_001,
    });
    return root.id;
  });
}

/**
 * Opens the agent activity panel from the composer activity bar — the ingress
 * that has no prior pane, so the header shows close and no back arrow.
 */
async function openActivityFromComposer(page: Page) {
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await waitForMockLiveSubscription(page, "agents", KIND_TYPING_INDICATOR);

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "agents",
      pubkey,
    });
  }, AGENT_PUBKEY);

  const trigger = page.getByTestId("bot-activity-composer-trigger");
  await expect(trigger).toBeVisible();
  await trigger.click();
  const item = page.getByTestId(`bot-activity-composer-item-${AGENT_PUBKEY}`);
  await expect(item).toBeVisible();
  await item.click({ force: true });
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
}

/**
 * There is one covered slot, so at most one drawer overlay may be in the DOM.
 * Asserts on overlays rather than the drawer surfaces because the overlay is
 * what makes the channel unreachable — two of them stacked is the failure the
 * user would actually feel.
 */
async function expectExactlyOneCoverDrawer(
  page: Page,
  expected: "agent-activity-drawer" | "focus-thread-drawer",
) {
  const others = ["agent-activity-drawer", "focus-thread-drawer"].filter(
    (testId) => testId !== expected,
  );
  await expect(page.getByTestId(`${expected}-overlay`)).toHaveCount(1);
  for (const testId of others) {
    await expect(page.getByTestId(`${testId}-overlay`)).toHaveCount(0);
  }
  await expect(page.getByTestId("channel-drop-zone")).toHaveAttribute(
    "inert",
    "",
  );
}

/**
 * Focus must end inside the drawer that won the slot.
 *
 * This is a positive check, not the regression guard: the covered channel is
 * `inert`, so a wrongly-restored focus into it is silently refused by the
 * browser and lands on `<body>` instead of visibly stealing focus. The
 * discriminating assertions for the handoff live in
 * `CoverDrawerFocusHandoff.test.mjs`, which drives the primitive directly.
 * Polls because both the successor's capture and the loser's deferred restore
 * land asynchronously.
 */
async function expectFocusInside(page: Page, testId: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (currentTestId) =>
          document
            .querySelector(`[data-testid="${currentTestId}"]`)
            ?.contains(document.activeElement) ?? false,
        testId,
      ),
    )
    .toBe(true);
}

/**
 * Opens a thread while activity covers the channel.
 *
 * The covered channel is inert, so its thread summaries cannot be clicked. A
 * `messageId` deep link reaches the same place: `useChannelRouteTarget` closes
 * the agent session and opens the thread in one navigation, which is exactly the
 * open-over-open transition under test — no closed intermediate state.
 *
 * The router uses hash history, so the param has to be written into the hash
 * fragment (see the same technique in `scroll-history.spec.ts`); rewriting
 * `location.search` would leave the router none the wiser.
 */
async function openThreadByMessageLink(page: Page, threadHeadId: string) {
  await page.evaluate((targetId) => {
    const hash = window.location.hash.replace(/^#/, "") || "/";
    const [path, query = ""] = hash.split("?");
    const params = new URLSearchParams(query);
    params.set("messageId", targetId);
    window.history.pushState(
      {},
      "",
      `${window.location.pathname}#${path}?${params.toString()}`,
    );
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, threadHeadId);
}

/**
 * Opens activity while a thread covers the channel, from the thread composer's
 * own activity bar — the one ingress that is reachable while the channel behind
 * is inert, so the thread never closes first.
 */
async function openActivityFromThreadComposer(
  page: Page,
  threadHeadId: string,
) {
  await page.evaluate(
    ({ currentThreadHeadId, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
        channelName: "agents",
        pubkey,
        threadHeadId: currentThreadHeadId,
      });
    },
    { currentThreadHeadId: threadHeadId, pubkey: AGENT_PUBKEY },
  );

  const drawer = page.getByTestId("focus-thread-drawer");
  const trigger = drawer.getByTestId("bot-activity-composer-trigger");
  await expect(trigger).toBeVisible();
  await trigger.click();
  const item = page.getByTestId(`bot-activity-composer-item-${AGENT_PUBKEY}`);
  await expect(item).toBeVisible();
  await item.click({ force: true });
}

/**
 * The default mock bridge already seeds alice as an agent in `#agents`, which
 * is what makes her eligible for the composer activity bar once she types.
 * Re-seeding her through `managedAgents` instead *replaces* that relay-agent
 * row with a managed one that is not in the channel's working-agent set, so the
 * trigger never renders — use the default seed.
 */
test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("agent activity covers the channel at wide viewports", async ({
  page,
}) => {
  await page.setViewportSize(WIDE_VIEWPORT);
  await page.goto("/");
  await openActivityFromComposer(page);

  const channel = page.getByTestId("channel-drop-zone");
  const drawer = page.getByTestId("agent-activity-drawer");
  const panel = page.getByTestId("agent-session-thread-panel");

  // Covering, not splitting: the panel lives inside the drawer, the channel is
  // inert behind it, and there is no split pane to resize.
  await expect(drawer).toBeVisible();
  await expect(drawer.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expect(channel).toHaveAttribute("inert", "");
  await expect(
    page.getByTestId("right-auxiliary-pane-resize-handle"),
  ).toHaveCount(0);

  // Activity never offers the thread's focus/split switch.
  await expect(page.getByTestId("thread-view-mode-toggle")).toHaveCount(0);

  // The drawer owns the entrance, so the panel must not slide too — a second
  // animation inside a moving container compounds into a double slide.
  await expect(panel).not.toHaveClass(/buzz-side-panel-enter/);

  // Wide enough to read a transcript: the drawer takes the channel content
  // area less the sliver, so it is far wider than the split pane it replaces.
  const drawerWidth = (await drawer.boundingBox())?.width ?? 0;
  expect(drawerWidth).toBeGreaterThan(700);

  // The drawer captures focus, and the panel keeps its close affordance.
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean(
          document
            .querySelector('[data-testid="agent-activity-drawer"]')
            ?.contains(document.activeElement),
        ),
      ),
    )
    .toBe(true);
  await expect(page.getByTestId("agent-session-back")).toHaveCount(0);
  await expect(page.getByTestId("auxiliary-panel-close")).toBeVisible();

  // Escape leaves — and the settings menu still gets its own press first.
  await page.getByTestId("agent-session-settings-menu-trigger").click();
  await expect(page.getByTestId("agent-session-stop-turn")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("agent-session-stop-turn")).toHaveCount(0);
  await expect(drawer).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("agent-activity-drawer-overlay")).toHaveCount(
    0,
  );
  await expect(panel).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");

  // The scrim is the click target back to the channel.
  await openActivityFromComposer(page);
  await expect(drawer).toBeVisible();
  await page
    .getByTestId("agent-activity-drawer-scrim")
    .click({ position: { x: 24, y: 300 } });
  await expect(page.getByTestId("agent-activity-drawer-overlay")).toHaveCount(
    0,
  );
  await expect(channel).not.toHaveAttribute("inert", "");
});

test("cover drawers replace each other in both directions without stacking", async ({
  page,
}) => {
  await page.setViewportSize(WIDE_VIEWPORT);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.channels.threadViewMode", "focus");
  });
  await page.goto("/");
  const rootId = await seedThreadRoot(page);
  await openActivityFromComposer(page);

  const agentDrawer = page.getByTestId("agent-activity-drawer");
  const threadDrawer = page.getByTestId("focus-thread-drawer");
  await expect(agentDrawer).toBeVisible();
  await expectExactlyOneCoverDrawer(page, "agent-activity-drawer");

  // Direction 1: thread opens over activity, with no closed intermediate. The
  // covered channel is inert so its thread summaries can't be clicked, but a
  // message link resolves through the same route-target handler, which clears
  // the agent session and opens the thread in one navigation.
  await openThreadByMessageLink(page, rootId);
  await expect(threadDrawer).toBeVisible();
  await expect(agentDrawer).toHaveCount(0);
  await expectExactlyOneCoverDrawer(page, "focus-thread-drawer");
  // The replaced surface's param is gone, not merely outranked.
  await expect(page).not.toHaveURL(/agentSession=/);
  await expect(page).toHaveURL(new RegExp(`thread=${rootId}`));
  await expectFocusInside(page, "focus-thread-drawer");

  // Direction 2: activity opens over the thread, again with no closed
  // intermediate — the thread drawer's own composer activity bar is live while
  // it covers, and its trigger calls the same open handler.
  await openActivityFromThreadComposer(page, rootId);
  await expect(agentDrawer).toBeVisible();
  await expect(threadDrawer).toHaveCount(0);
  await expectExactlyOneCoverDrawer(page, "agent-activity-drawer");
  await expect(page).not.toHaveURL(new RegExp(`thread=${rootId}`));
  await expect(page).toHaveURL(/agentSession=/);
  await expectFocusInside(page, "agent-activity-drawer");
});

test("narrow viewports keep the existing activity presentation", async ({
  page,
}) => {
  await page.setViewportSize(NARROW_VIEWPORT);
  await page.goto("/");
  await openActivityFromComposer(page);

  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await expect(page.getByTestId("agent-activity-drawer")).toHaveCount(0);
  await expect(page.getByTestId("agent-activity-drawer-overlay")).toHaveCount(
    0,
  );
  await expect(page.getByTestId("agent-activity-drawer-scrim")).toHaveCount(0);
  // Below the breakpoint the channel is replaced rather than covered, so the
  // pane it would be made inert behind is not rendered at all — and nothing
  // else on the page is inert either.
  await expect(page.getByTestId("channel-drop-zone")).toHaveCount(0);
  expect(await page.locator("[inert]").count()).toBe(0);
});
