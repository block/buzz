import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const BESTIE_PUBKEY =
  "be571e0000000000000000000000000000000000000000000000000000000000";
const OWNER_PUBKEY = "deadbeef".repeat(8);
const RELAY_A = "ws://localhost:3000";
const COMMUNITY_A = {
  addedAt: "2026-01-01T00:00:00.000Z",
  id: "bestie-community-a",
  name: "Alpha",
  relayUrl: RELAY_A,
};
const COMMUNITY_B = {
  addedAt: "2026-01-02T00:00:00.000Z",
  id: "bestie-community-b",
  name: "Bravo",
  relayUrl: "ws://localhost:3001",
};

const bestie = {
  avatarUrl: null,
  name: "Bestie",
  personaId: "builtin:bestie",
  pubkey: BESTIE_PUBKEY,
  relayUrl: RELAY_A,
  status: "running" as const,
};
const bestieInCommunityB = {
  ...bestie,
  relayUrl: COMMUNITY_B.relayUrl,
};

async function seedCommunities(
  page: import("@playwright/test").Page,
  activeId = COMMUNITY_A.id,
) {
  await page.addInitScript(
    ({ active, communities }) => {
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify(communities),
      );
      window.localStorage.setItem("buzz-active-community-id", active);
    },
    { active: activeId, communities: [COMMUNITY_A, COMMUNITY_B] },
  );
}

function channelIdFromUrl(url: string) {
  const channelId = new URL(url).hash.match(/^#\/channels\/([^?]+)/)?.[1];
  if (!channelId) throw new Error(`Expected a channel route, got ${url}`);
  return channelId;
}

test("the enabled Bestie experiment adds a direct-message entry below Agents", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.goto("/");

  const agentsEntry = page.getByTestId("open-agents-view");
  const bestieEntry = page.getByTestId("open-bestie-dm");
  await expect(bestieEntry).toBeVisible();
  await expect(bestieEntry).toContainText("Bestie");

  const [agentsBox, bestieBox] = await Promise.all([
    agentsEntry.boundingBox(),
    bestieEntry.boundingBox(),
  ]);
  expect(agentsBox).not.toBeNull();
  expect(bestieBox).not.toBeNull();
  expect(bestieBox?.y).toBeGreaterThan(agentsBox?.y ?? 0);

  await bestieEntry.click();
  await expect(page.getByTestId("chat-title")).toHaveText("Bestie");
});

test("the disabled Bestie experiment does not mount the sidebar entry", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.addInitScript((key) => {
    const overrides = JSON.parse(
      window.localStorage.getItem(key) ?? "{}",
    ) as Record<string, boolean>;
    overrides.bestie = false;
    window.localStorage.setItem(key, JSON.stringify(overrides));
  }, FEATURE_OVERRIDES_STORAGE_KEY);
  await page.goto("/");

  await expect(page.getByTestId("open-bestie-dm")).toHaveCount(0);
  await expect(page.getByTestId("open-bestie-panel")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+1");
  await expect(page.getByTestId("bestie-chat-popover")).toHaveCount(0);
});

test("the app-level avatar and command shortcut share one Bestie conversation", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.goto("/");

  const trigger = page.getByTestId("open-bestie-panel");
  const topChrome = page.getByTestId("app-top-chrome");
  await expect(trigger).toBeVisible();
  await expect(trigger).toHaveAccessibleName("Open Bestie chat");

  const [triggerBox, chromeBox] = await Promise.all([
    trigger.boundingBox(),
    topChrome.boundingBox(),
  ]);
  expect(triggerBox).not.toBeNull();
  expect(chromeBox).not.toBeNull();
  expect(triggerBox?.x).toBeGreaterThan((chromeBox?.x ?? 0) + 800);
  expect(triggerBox?.y).toBeGreaterThanOrEqual(chromeBox?.y ?? 0);
  expect((triggerBox?.y ?? 0) + (triggerBox?.height ?? 0)).toBeLessThanOrEqual(
    (chromeBox?.y ?? 0) + (chromeBox?.height ?? 0),
  );

  const popover = page.getByTestId("bestie-chat-popover");
  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeVisible();
  await expect(popover).toContainText("Bestie");
  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeHidden();
  await trigger.click();
  await expect(popover).toBeVisible();

  const composer = popover.getByTestId("message-composer");
  const editor = composer.locator('[contenteditable="true"]');
  await expect(editor).toBeEditable();
  await editor.fill("Keep this decision close at hand.");
  await composer.getByRole("button", { name: "Send" }).click();
  await expect(popover.getByTestId("message-row").last()).toContainText(
    "Keep this decision close at hand.",
  );

  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeHidden();
  await page.getByTestId("channel-general").click();
  const channelEditor = page
    .getByTestId("message-composer")
    .locator('[contenteditable="true"]');
  await channelEditor.focus();
  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeVisible();
  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeHidden();

  await page.goto("/#/settings");
  await expect(trigger).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeVisible();
  await page.keyboard.press("ControlOrMeta+1");
  await expect(popover).toBeHidden();
});

test("a failed Bestie open stays actionable and Retry restores real history", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [bestie],
    openDmErrors: [null, "relay offline", null],
  });
  await page.goto("/");

  await page.getByTestId("open-bestie-dm").click();
  await expect(page.getByTestId("chat-title")).toHaveText("Bestie");
  const composer = page.getByTestId("message-composer");
  await composer.locator('[contenteditable="true"]').fill("Existing history");
  await composer.getByRole("button", { name: "Send" }).click();
  await expect(page.getByTestId("message-row").last()).toContainText(
    "Existing history",
  );
  await page.getByTestId("channel-general").click();

  await page.getByTestId("open-bestie-panel").click();
  const popover = page.getByTestId("bestie-chat-popover");
  await expect(popover.getByRole("alert")).toContainText(
    "Couldn't load your conversation with Bestie.",
  );
  await expect(popover.locator('[contenteditable="true"]')).toHaveCount(0);

  await popover.getByRole("button", { name: "Retry" }).click();
  await expect(popover.getByTestId("bestie-chat-transcript")).toContainText(
    "Existing history",
  );
  await expect(popover.locator('[contenteditable="true"]')).toBeEditable();
});

test("viewing an unread Bestie DM in the popover advances its canonical read marker", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.goto("/");

  await page.getByTestId("open-bestie-dm").click();
  await expect(page.getByTestId("chat-title")).toHaveText("Bestie");
  const bestieChannelId = channelIdFromUrl(page.url());
  await page.getByTestId("channel-general").click();
  const createdAt = Math.floor(Date.now() / 1_000) + 1;
  await page.evaluate(
    ({ channelId, content, createdAt, pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelId,
        content,
        createdAt,
        pubkey,
      }),
    {
      channelId: bestieChannelId,
      content: "Unread Bestie guidance",
      createdAt,
      pubkey: BESTIE_PUBKEY,
    },
  );
  await expect
    .poll(() =>
      page.evaluate(
        (channelId) =>
          window.__BUZZ_E2E_COMMAND_LOG__?.some(
            (entry) =>
              entry.command === "observed_unread_ingest" &&
              (
                entry.payload as {
                  request?: { events?: Array<{ channelId?: string }> };
                }
              )?.request?.events?.some(
                (event) => event.channelId === channelId,
              ),
          ),
        bestieChannelId,
      ),
    )
    .toBe(true);
  await page.evaluate(() => {
    window.__BUZZ_E2E_COMMAND_LOG__ = [];
  });

  await page.getByTestId("open-bestie-panel").click();
  const popover = page.getByTestId("bestie-chat-popover");
  await expect(popover.getByTestId("bestie-chat-transcript")).toContainText(
    "Unread Bestie guidance",
  );
  await expect
    .poll(() =>
      page.evaluate(
        ({ channelId, readAt }) =>
          window.__BUZZ_E2E_COMMAND_LOG__?.some(
            (entry) =>
              entry.command === "observed_unread_ingest" &&
              (
                entry.payload as {
                  request?: {
                    markers?: Array<{
                      contextId?: string;
                      readAt?: number | null;
                    }>;
                  };
                }
              )?.request?.markers?.some(
                (marker) =>
                  marker.contextId === channelId &&
                  (marker.readAt ?? 0) >= readAt,
              ),
          ),
        { channelId: bestieChannelId, readAt: createdAt },
      ),
    )
    .toBe(true);
});

test("a delayed Bestie open is scoped to its rendered community and signer", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { managedAgents: [bestie], openDmDelayMs: 1_000 },
    { skipCommunitySeed: true },
  );
  await seedCommunities(page);
  await page.goto("/");

  await page.getByTestId("open-bestie-dm").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
            (entry) => entry.command === "open_dm",
          )?.payload,
      ),
    )
    .toBeTruthy();
  const openDmPayload = await page.evaluate(
    () =>
      window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
        (entry) => entry.command === "open_dm",
      )?.payload,
  );
  expect(openDmPayload).toMatchObject({
    expectedRelayUrl: RELAY_A,
    expectedSignerPubkey: OWNER_PUBKEY,
    pubkeys: [BESTIE_PUBKEY],
  });

  await page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.localStorage.getItem("buzz-active-community-id"),
      ),
    )
    .toBe(COMMUNITY_B.id);

  await page.waitForTimeout(1_150);
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
  await expect(page.getByTestId("open-bestie-dm")).toHaveCount(0);
});

test("emoji and overflow expand from one measured message toolbar surface", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const messageId = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    return emit({
      channelName: "general",
      content: "Bloom architecture check",
      id: "c".repeat(64),
    }).id;
  });

  const row = page.locator(`[data-message-id="${messageId}"]`);
  await expect(row).toContainText("Bloom architecture check");
  await row.hover();
  const container = row.getByTestId(
    `message-action-bloom-container-${messageId}`,
  );
  const closedBox = await container.boundingBox();
  expect(closedBox).not.toBeNull();
  await row.getByTestId(`react-message-${messageId}`).click();
  await expect(
    row.getByTestId(`reaction-bloom-panel-${messageId}`),
  ).toBeVisible();
  await expect(container).toHaveAttribute("popover", "manual");
  await expect(
    row.locator(`[data-testid="message-action-bloom-container-${messageId}"]`),
  ).toHaveCount(1);
  const reactionBox = await container.boundingBox();
  expect(reactionBox).not.toBeNull();
  expect(
    Math.abs(
      (closedBox?.x ?? 0) +
        (closedBox?.width ?? 0) -
        ((reactionBox?.x ?? 0) + (reactionBox?.width ?? 0)),
    ),
  ).toBeLessThanOrEqual(1);

  await page.keyboard.press("Escape");
  await row.getByTestId(`more-actions-${messageId}`).click();
  await expect(
    row.getByTestId(`more-actions-panel-${messageId}`),
  ).toBeVisible();
  const overflowBox = await container.boundingBox();
  expect(overflowBox).not.toBeNull();
  expect(
    Math.abs(
      (closedBox?.x ?? 0) +
        (closedBox?.width ?? 0) -
        ((overflowBox?.x ?? 0) + (overflowBox?.width ?? 0)),
    ),
  ).toBeLessThanOrEqual(1);
});

test("a message can be handed to Bestie with an optional note", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents: [bestie] });
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const messageId = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    return emit({
      channelName: "general",
      content:
        "Please fold this launch decision into tomorrow's priorities and name one owner.",
      id: "d".repeat(64),
    }).id;
  });

  const row = page.locator(`[data-message-id="${messageId}"]`);
  await expect(row).toContainText("launch decision");
  await row.hover();
  await row.getByTestId(`send-to-bestie-${messageId}`).click();

  const actionBar = row.getByTestId(`message-action-bar-${messageId}`);
  const popover = page.getByTestId(`bestie-popover-${messageId}`);
  await expect(actionBar).toHaveAttribute("data-bloom-surface", "bestie");
  await expect(popover).toBeVisible();
  await expect(popover).toContainText("Bestie");
  await expect(popover).toContainText("Please fold this launch decision");

  const snapshot = popover.getByTestId(`bestie-message-snapshot-${messageId}`);
  const snapshotBody = popover.getByTestId(
    `bestie-message-snapshot-body-${messageId}`,
  );
  const [popoverBox, snapshotBox, snapshotBodyBox] = await Promise.all([
    popover.boundingBox(),
    snapshot.boundingBox(),
    snapshotBody.boundingBox(),
  ]);
  expect(popoverBox).not.toBeNull();
  expect(snapshotBox).not.toBeNull();
  expect(snapshotBodyBox).not.toBeNull();
  if (popoverBox && snapshotBox && snapshotBodyBox) {
    expect(popoverBox.width).toBeLessThanOrEqual(328);
    expect(snapshotBox.width / (popoverBox.width - 32)).toBeCloseTo(0.75, 1);
    expect(Math.abs(snapshotBox.x - (popoverBox.x + 16))).toBeLessThanOrEqual(
      1,
    );
    expect(snapshotBox.height).toBeLessThan(64);
    expect(snapshotBodyBox.height).toBeLessThanOrEqual(14);
  }

  const composer = popover.getByTestId("message-composer");
  await expect(composer.getByRole("button", { name: "Send" })).toBeEnabled();
  await composer
    .locator('[contenteditable="true"]')
    .fill("Make sure product and engineering agree on the owner.");
  await composer.getByRole("button", { name: "Send" }).click();

  await expect(popover).toBeHidden();
  await page.getByTestId("open-bestie-dm").click();
  const sentMessage = page.getByTestId("message-row").last();
  await expect(sentMessage).toContainText(
    "Make sure product and engineering agree on the owner.",
  );
  await expect(sentMessage).toContainText("Open original message");
});

test("a Bestie handoff fails closed across a community switch and preserves its draft", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      managedAgents: [bestie, bestieInCommunityB],
      sendMessageDelayMs: 1_000,
    },
    { skipCommunitySeed: true },
  );
  await seedCommunities(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const messageId = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    return emit({
      channelName: "general",
      content: "Keep this handoff in its original community",
      id: "e".repeat(64),
    }).id;
  });

  const draft = "This note must stay in Alpha.";
  const row = page.locator(`[data-message-id="${messageId}"]`);
  await row.hover();
  await row.getByTestId(`send-to-bestie-${messageId}`).click();
  const popover = page.getByTestId(`bestie-popover-${messageId}`);
  const editor = popover.locator('[contenteditable="true"]');
  await editor.fill(draft);
  await popover.getByRole("button", { name: "Send" }).click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMAND_LOG__?.findLast(
            (entry) => entry.command === "send_channel_message",
          )?.payload,
      ),
    )
    .toMatchObject({
      content: expect.stringContaining(draft),
      expectedRelayUrl: RELAY_A,
      expectedSignerPubkey: OWNER_PUBKEY,
    });

  await page.getByTestId(`community-rail-button-${COMMUNITY_B.id}`).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.localStorage.getItem("buzz-active-community-id"),
      ),
    )
    .toBe(COMMUNITY_B.id);
  await page.waitForTimeout(1_150);

  // The two fixtures intentionally resolve to the same participant-derived
  // DM id. An unscoped send would therefore appear in both tenant views.
  await page
    .getByTestId("sidebar-channel-content")
    .getByRole("button", { name: /Bestie/ })
    .first()
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("Bestie");
  await expect(page.getByTestId("message-timeline")).not.toContainText(draft);

  await page.getByTestId(`community-rail-button-${COMMUNITY_A.id}`).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.localStorage.getItem("buzz-active-community-id"),
      ),
    )
    .toBe(COMMUNITY_A.id);
  await page
    .getByTestId("sidebar-channel-content")
    .getByRole("button", { name: /Bestie/ })
    .first()
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("Bestie");
  await expect(page.getByTestId("message-timeline")).not.toContainText(draft);

  await page.getByTestId("channel-general").click();
  await page.evaluate((id) => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    emit({
      channelName: "general",
      content: "Keep this handoff in its original community",
      id,
    });
  }, messageId);
  const restoredRow = page.locator(`[data-message-id="${messageId}"]`);
  await restoredRow.hover();
  await restoredRow.getByTestId(`send-to-bestie-${messageId}`).click();
  const restoredPopover = page.getByTestId(`bestie-popover-${messageId}`);
  await expect(restoredPopover.locator('[contenteditable="true"]')).toHaveText(
    draft,
  );
  await page.evaluate(() => {
    if (window.__BUZZ_E2E__?.mock) {
      window.__BUZZ_E2E__.mock.sendMessageDelayMs = 0;
    }
  });
  await restoredPopover.getByRole("button", { name: "Send" }).click();
  await expect(restoredPopover).toBeHidden();
  await page.getByTestId("open-bestie-dm").click();
  await expect(page.getByTestId("message-timeline")).toContainText(draft);
});
