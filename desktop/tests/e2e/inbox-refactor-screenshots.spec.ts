/**
 * Screenshots documenting the focused Inbox behavior for PR #2045.
 *
 * These replace the hand-generated shots posted earlier in that PR, which went
 * stale twice: once when the surface was briefly renamed to "Activity", and
 * again when the Projects filter landed. Keeping them in a spec means the next
 * round regenerates instead of drifting.
 *
 * Run: pnpm build:e2e && pnpm exec playwright test --project=smoke \
 *        tests/e2e/inbox-refactor-screenshots.spec.ts
 * Output: test-results/inbox-refactor/
 */
import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/inbox-refactor";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const DM_CHANNEL_ID = "f48efb06-0c93-5025-aac9-2e646bb6bfa8";

// Mock bridge default pubkey — must match DEFAULT_MOCK_PUBKEY in bridge.ts.
const MOCK_PUBKEY = "deadbeef".repeat(8);
const DRAFT_STORE_KEY = `buzz-drafts.v1:${MOCK_PUBKEY}`;
const FONT_SIZE_STORAGE_KEY = "buzz.appearance.fontSize";
const CONVERSATION_DENSITY_STORAGE_KEY = "buzz.appearance.conversationDensity";

// Fixed timestamps so draft ordering renders deterministically.
const DRAFT_CREATED_AT_1 = "2026-07-01T10:00:00.000Z";
const DRAFT_CREATED_AT_2 = "2026-07-02T14:30:00.000Z";

type FontSize = "smaller" | "default" | "larger";
type ConversationDensity = "compact" | "comfortable" | "spacious";

async function seedConversationPreferences(
  page: Page,
  fontSize: FontSize,
  density: ConversationDensity,
) {
  await page.addInitScript(
    ({ densityKey, densityValue, fontSizeKey, fontSizeValue }) => {
      window.localStorage.setItem(densityKey, densityValue);
      window.localStorage.setItem(fontSizeKey, fontSizeValue);
    },
    {
      densityKey: CONVERSATION_DENSITY_STORAGE_KEY,
      densityValue: density,
      fontSizeKey: FONT_SIZE_STORAGE_KEY,
      fontSizeValue: fontSize,
    },
  );
}

async function applyConversationPreferences(
  page: Page,
  fontSize: FontSize,
  density: ConversationDensity,
) {
  await page.evaluate(
    ({ densityKey, densityValue, fontSizeKey, fontSizeValue }) => {
      const update = (key: string, value: string) => {
        const oldValue = window.localStorage.getItem(key);
        window.localStorage.setItem(key, value);
        window.dispatchEvent(
          new StorageEvent("storage", {
            key,
            newValue: value,
            oldValue,
            storageArea: window.localStorage,
          }),
        );
      };
      update(densityKey, densityValue);
      update(fontSizeKey, fontSizeValue);
    },
    {
      densityKey: CONVERSATION_DENSITY_STORAGE_KEY,
      densityValue: density,
      fontSizeKey: FONT_SIZE_STORAGE_KEY,
      fontSizeValue: fontSize,
    },
  );
}

type MockFeedWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    createdAt?: number;
    id?: string;
    parentEventId?: string;
    pubkey?: string;
  }) => {
    content: string;
    created_at: number;
    id: string;
    kind: number;
    pubkey: string;
    tags: string[][];
  };
  __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
    category: "mention" | "needs_action" | "activity" | "agent_activity";
    channel_id: string | null;
    channel_name: string;
    channel_type?: string | null;
    content: string;
    created_at: number;
    id: string;
    kind: number;
    pubkey: string;
    tags: string[][];
  }) => void;
};

/**
 * The mock community is seeded without a pubkey, which initDraftStore needs on
 * startup. Init scripts run in registration order, so this patches the seed
 * installMockBridge already wrote.
 */
async function patchCommunityPubkey(page: import("@playwright/test").Page) {
  await page.addInitScript(
    ({ pubkey }) => {
      const raw = window.localStorage.getItem("buzz-communities");
      const communities = raw
        ? (JSON.parse(raw) as Array<Record<string, unknown>>)
        : [];
      if (communities[0]) {
        communities[0].pubkey = pubkey;
        window.localStorage.setItem(
          "buzz-communities",
          JSON.stringify(communities),
        );
      }
    },
    { pubkey: MOCK_PUBKEY },
  );
}

/** Two active drafts so the Drafts filter renders its count badge. */
async function seedTwoDrafts(page: import("@playwright/test").Page) {
  await page.addInitScript(
    ({ storeKey, channelId, agentsChannelId, createdAt1, createdAt2 }) => {
      const draft = (content: string, channel: string, timestamp: string) => ({
        content,
        selectionStart: content.length,
        selectionEnd: content.length,
        channelId: channel,
        createdAt: timestamp,
        updatedAt: timestamp,
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        status: "active" as const,
      });

      window.localStorage.setItem(
        storeKey,
        JSON.stringify({
          [`channel:${channelId}`]: draft(
            "Drafting the release notes — will post once the checklist is signed off.",
            channelId,
            createdAt1,
          ),
          [`channel:${agentsChannelId}`]: draft(
            "Following up on the harness rollout.",
            agentsChannelId,
            createdAt2,
          ),
        }),
      );
    },
    {
      storeKey: DRAFT_STORE_KEY,
      channelId: GENERAL_CHANNEL_ID,
      agentsChannelId: AGENTS_CHANNEL_ID,
      createdAt1: DRAFT_CREATED_AT_1,
      createdAt2: DRAFT_CREATED_AT_2,
    },
  );
}

/** Wait for the live mock-feed helpers the seeding below depends on. */
async function waitForMockFeedHelpers(page: import("@playwright/test").Page) {
  await expect(page.getByTestId("home-inbox-list")).toBeVisible({
    timeout: 10_000,
  });
  await page.waitForFunction(() => {
    const win = window as MockFeedWindow;
    return (
      typeof win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ === "function"
    );
  });
}

test.describe("inbox refactor screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => console.error("PAGE ERROR:", err.message));
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 300));
      }
    });
  });

  test("01 — filter menu with Reminders/Drafts divider and counts", async ({
    page,
  }) => {
    await patchCommunityPubkey(page);
    await seedTwoDrafts(page);
    await installMockBridge(page, { mode: "mock" });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("home-inbox")).toBeVisible({
      timeout: 10_000,
    });

    await page.getByTestId("inbox-filter-trigger").click();
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(
      menu.getByRole("menuitemradio", { name: "Projects" }),
    ).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({ path: `${SHOTS}/01-current-filters.png` });
  });

  test("02 — Inbox label, inbox icon, and overflow controls", async ({
    page,
  }) => {
    await installMockBridge(page, { mode: "mock" });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("home-inbox")).toBeVisible({
      timeout: 10_000,
    });

    // The sidebar must be in frame — the label is the point of this shot.
    const inboxButton = page
      .getByTestId("sidebar-primary-menu")
      .getByRole("button", { name: "Inbox", exact: true });
    await expect(inboxButton).toBeVisible();
    // Inbox is a destination, not a notification tray, so it carries the inbox
    // glyph rather than a bell. Asserted because nothing else pins the icon.
    await expect(inboxButton.locator("svg.lucide-inbox")).toHaveCount(1);

    await page.getByTestId("inbox-options-trigger").click();
    await expect(page.getByText("Show unread only")).toBeVisible();
    await expect(page.getByText("Mark all as read")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({ path: `${SHOTS}/02-current-controls.png` });
  });

  test("03 — consecutive DMs group into one conversation row", async ({
    page,
  }) => {
    await installMockBridge(page, { mode: "mock" });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await waitForMockFeedHelpers(page);

    const dmIds = ["shot-dm-first", "shot-dm-second", "shot-dm-third"];
    await page.evaluate(
      ({ channelId, createdAt, ids, senderPubkey }) => {
        const win = window as MockFeedWindow;
        const emitMessage = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const pushFeedItem = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emitMessage || !pushFeedItem) {
          throw new Error("Mock bridge helpers are not installed.");
        }

        [
          "Quick update: the staging deploy is ready.",
          "I ran the smoke tests and everything passed.",
          "Can you take a final look before we release?",
        ].forEach((content, index) => {
          const event = emitMessage({
            channelName: "alice-tyler",
            content,
            createdAt: createdAt + index,
            id: ids[index],
            pubkey: senderPubkey,
          });
          pushFeedItem({
            category: "activity",
            channel_id: channelId,
            channel_name: "alice-tyler",
            channel_type: null,
            content: event.content,
            created_at: event.created_at,
            id: event.id,
            kind: event.kind,
            pubkey: event.pubkey,
            tags: event.tags,
          });
        });
      },
      {
        channelId: DM_CHANNEL_ID,
        createdAt: Math.floor(Date.now() / 1000),
        ids: dmIds,
        senderPubkey: TEST_IDENTITIES.alice.pubkey,
      },
    );

    const firstDmRow = page.getByTestId(`home-inbox-item-${dmIds[0]}`);
    await expect(firstDmRow).toBeVisible();
    // The grouping claim: three DMs, one row.
    await expect(page.getByTestId(`home-inbox-item-${dmIds[1]}`)).toHaveCount(
      0,
    );
    await expect(page.getByTestId(`home-inbox-item-${dmIds[2]}`)).toHaveCount(
      0,
    );

    await firstDmRow.click();
    const detail = page.getByTestId("home-inbox-detail");
    await expect(detail.getByRole("heading")).toHaveText("DM with alice");
    await expect(detail.getByTestId("message-unread-divider")).toBeVisible();
    await waitForAnimations(page);

    await page.screenshot({ path: `${SHOTS}/03-grouped-dms.png` });
  });

  test("04 — thread opens at the oldest unread reply", async ({ page }) => {
    await seedConversationPreferences(page, "default", "comfortable");
    await installMockBridge(page, { mode: "mock" });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await waitForMockFeedHelpers(page);

    const replyIds = [
      "shot-reply-first",
      "shot-reply-second",
      "shot-reply-third",
    ];
    await page.evaluate(
      ({ agentPubkeys, channelId, currentPubkey, ids }) => {
        const win = window as MockFeedWindow;
        const emitMessage = win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        const pushFeedItem = win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
        if (!emitMessage || !pushFeedItem) {
          throw new Error("Mock bridge helpers are not installed.");
        }

        const createdAt = Math.floor(Date.now() / 1000) + 60;
        const root = emitMessage({
          channelName: "general",
          content: "Release checklist for the desktop build",
          createdAt: createdAt - 10,
          id: "shot-thread-root",
          pubkey: currentPubkey,
        });

        [
          "Started on the changelog — first pass is up.",
          "Verified the signed build on macOS.",
          "One open question on the version suffix.",
        ].forEach((content, index) => {
          const event = emitMessage({
            channelName: "general",
            content,
            createdAt: createdAt + index,
            id: ids[index],
            parentEventId: root.id,
            pubkey: agentPubkeys[index % agentPubkeys.length],
          });
          pushFeedItem({
            category: "activity",
            channel_id: channelId,
            channel_name: "general",
            channel_type: "stream",
            content: event.content,
            created_at: event.created_at,
            id: event.id,
            kind: event.kind,
            pubkey: event.pubkey,
            tags: event.tags,
          });
        });
      },
      {
        agentPubkeys: [
          TEST_IDENTITIES.alice.pubkey,
          TEST_IDENTITIES.charlie.pubkey,
        ],
        channelId: GENERAL_CHANNEL_ID,
        currentPubkey: MOCK_PUBKEY,
        ids: replyIds,
      },
    );

    const firstUnreadRow = page.getByTestId(`home-inbox-item-${replyIds[0]}`);
    await expect(firstUnreadRow).toBeVisible();
    const listPreview = firstUnreadRow.locator(".inbox-preview-markdown");
    await expect(listPreview).toHaveCSS("font-size", "14px");
    await expect(listPreview).toHaveCSS("line-height", "20px");
    await firstUnreadRow.click();

    const detail = page.getByTestId("home-inbox-detail");
    // The anchoring claim: full thread present, selection on the oldest unread.
    await expect(detail).toContainText(
      "Release checklist for the desktop build",
    );
    await expect(detail.getByTestId("message-unread-divider")).toBeVisible();
    await expect(page.getByTestId("home-inbox-selected-message")).toContainText(
      "Started on the changelog — first pass is up.",
    );

    const selectedMessage = page.getByTestId("home-inbox-selected-message");
    const selectedAuthor = selectedMessage.getByTestId("message-author");
    const selectedBody = selectedMessage.locator(".message-markdown").first();
    const selectedTimestamp = selectedMessage.getByTestId(
      "inbox-message-timestamp",
    );
    const remainingListPreview = page
      .locator("[data-testid^='home-inbox-item-']")
      .locator(".inbox-preview-markdown")
      .first();
    const composerInput = page.getByTestId("message-input");
    const readConversationMetrics = () =>
      Promise.all([
        remainingListPreview.evaluate((element) => {
          const style = window.getComputedStyle(element);
          return {
            fontSize: style.fontSize,
            lineHeight: style.lineHeight,
          };
        }),
        selectedMessage.evaluate((element) => {
          const style = window.getComputedStyle(element);
          return {
            paddingBottom: style.paddingBottom,
            paddingTop: style.paddingTop,
          };
        }),
        selectedMessage.evaluate((element) => {
          const header = element.querySelector<HTMLElement>(
            "[data-testid='message-header']",
          );
          const body = element.querySelector<HTMLElement>(
            "[data-testid='message-body']",
          );
          if (!header || !body) {
            throw new Error("Inbox message spacing geometry is missing");
          }
          return (
            body.getBoundingClientRect().top -
            header.getBoundingClientRect().bottom
          );
        }),
        selectedAuthor.evaluate((element) => {
          const style = window.getComputedStyle(element);
          return {
            fontSize: style.fontSize,
            lineHeight: style.lineHeight,
          };
        }),
        selectedBody.evaluate((element) => {
          const style = window.getComputedStyle(element);
          return {
            fontSize: style.fontSize,
            lineHeight: style.lineHeight,
          };
        }),
        selectedTimestamp.evaluate((element) => {
          const style = window.getComputedStyle(element);
          return {
            fontSize: style.fontSize,
            lineHeight: style.lineHeight,
          };
        }),
        composerInput.evaluate((element) => {
          const style = window.getComputedStyle(element);
          return {
            fontSize: style.fontSize,
            lineHeight: style.lineHeight,
          };
        }),
      ]);
    await expect
      .poll(readConversationMetrics)
      .toEqual([
        { fontSize: "14px", lineHeight: "20px" },
        { paddingBottom: "4px", paddingTop: "4px" },
        2,
        { fontSize: "14px", lineHeight: "16px" },
        { fontSize: "14px", lineHeight: "20px" },
        { fontSize: "12px", lineHeight: "16px" },
        { fontSize: "14px", lineHeight: "20px" },
      ]);
    await waitForAnimations(page);

    await page.screenshot({ path: `${SHOTS}/04-thread-context.png` });

    await applyConversationPreferences(page, "default", "compact");
    await expect
      .poll(readConversationMetrics)
      .toEqual([
        { fontSize: "14px", lineHeight: "20px" },
        { paddingBottom: "4px", paddingTop: "4px" },
        0,
        { fontSize: "14px", lineHeight: "16px" },
        { fontSize: "14px", lineHeight: "20px" },
        { fontSize: "12px", lineHeight: "16px" },
        { fontSize: "14px", lineHeight: "20px" },
      ]);

    await applyConversationPreferences(page, "smaller", "compact");
    await expect
      .poll(readConversationMetrics)
      .toEqual([
        { fontSize: "13px", lineHeight: "18.5714px" },
        { paddingBottom: "4px", paddingTop: "4px" },
        0,
        { fontSize: "13px", lineHeight: "14.8571px" },
        { fontSize: "13px", lineHeight: "18.5714px" },
        { fontSize: "11.1429px", lineHeight: "14.8571px" },
        { fontSize: "13px", lineHeight: "18.5714px" },
      ]);
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/05-thread-context-compact.png` });

    await applyConversationPreferences(page, "larger", "spacious");
    await expect
      .poll(readConversationMetrics)
      .toEqual([
        { fontSize: "15px", lineHeight: "21.4286px" },
        { paddingBottom: "8px", paddingTop: "8px" },
        4,
        { fontSize: "15px", lineHeight: "17.1429px" },
        { fontSize: "15px", lineHeight: "21.4286px" },
        { fontSize: "12.8571px", lineHeight: "17.1429px" },
        { fontSize: "15px", lineHeight: "21.4286px" },
      ]);
    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/06-thread-context-spacious.png` });

    await applyConversationPreferences(page, "default", "comfortable");

    await page.evaluate(() => {
      const isMac = /mac|iphone|ipad|ipod/i.test(navigator.platform);
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          code: "Equal",
          ctrlKey: !isMac,
          key: "+",
          metaKey: isMac,
          shiftKey: true,
        }),
      );
    });

    await expect
      .poll(async () => [
        await page.evaluate(() =>
          window
            .getComputedStyle(document.documentElement)
            .getPropertyValue("--buzz-type-rem")
            .trim(),
        ),
        ...(await readConversationMetrics()),
      ])
      .toEqual([
        "17.6px",
        { fontSize: "15.4px", lineHeight: "22px" },
        { paddingBottom: "4px", paddingTop: "4px" },
        2,
        { fontSize: "15.4px", lineHeight: "17.6px" },
        { fontSize: "15.4px", lineHeight: "22px" },
        { fontSize: "13.2px", lineHeight: "17.6px" },
        { fontSize: "15.4px", lineHeight: "22px" },
      ]);
  });
});
