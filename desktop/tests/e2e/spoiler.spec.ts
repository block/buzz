import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const IMAGE_SHA = "c".repeat(64);
const IMAGE_URL = "http://127.0.0.1:4173/buzz.svg";
const IMAGE_DESCRIPTOR = {
  url: IMAGE_URL,
  sha256: IMAGE_SHA,
  size: 646,
  type: "image/svg+xml",
  uploaded: Math.floor(Date.now() / 1000),
  thumb: IMAGE_URL,
  dim: "64x64",
  filename: "buzz.svg",
};
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

type MockFeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  created_at: number;
  channel_id: string | null;
  channel_name: string;
  tags: string[][];
  category: "mention" | "needs_action" | "activity" | "agent_activity";
};

type MockFeedWindow = Window &
  typeof globalThis & {
    __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: MockFeedItem) => MockFeedItem;
  };

async function installSpoilerBridge(
  page: Page,
  mock: Parameters<typeof installMockBridge>[1] = {},
) {
  await installMockBridge(page, {
    ...mock,
    uploadDescriptors: [IMAGE_DESCRIPTOR],
  });
}

test("no-selection spoiler applies to every composer paragraph", async ({
  page,
}) => {
  await installSpoilerBridge(page);
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  const paragraphs = [
    "First hidden paragraph",
    "Second hidden paragraph",
    "Third hidden paragraph",
  ];

  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    paragraphs.join("\n\n"),
  );
  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await expect(input.locator("p")).toHaveCount(paragraphs.length);

  await page.getByRole("button", { name: "Spoiler", exact: true }).click();

  await expect
    .poll(() =>
      input.evaluate(() =>
        Array.from(
          document.querySelectorAll(
            '[data-testid="message-input"] .buzz-spoiler[data-spoiler]',
          ),
          (node) => node.textContent,
        ),
      ),
    )
    .toEqual(paragraphs);
});

test("image attachments can be marked and sent as hidden spoilers", async ({
  page,
}) => {
  await installSpoilerBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByRole("button", { name: "Attach image" }).click();

  const composer = page.getByTestId("message-composer");
  await expect(composer.getByAltText("Attachment cccc")).toBeVisible();

  await page.getByRole("button", { name: "Spoiler", exact: true }).click();
  await expect(composer.locator("[data-composer-media-spoiler]")).toBeVisible();

  await page.getByTestId("send-message").click();

  const lastMessage = page.getByTestId("message-row").last();
  const spoilerBlock = lastMessage.locator(".buzz-spoiler--block");
  await expect(spoilerBlock).toBeVisible();
  await expect(spoilerBlock).toHaveAttribute("data-revealed", "false");
  await expect(spoilerBlock.locator("[data-block-media] img")).toHaveAttribute(
    "src",
    IMAGE_URL,
  );

  await spoilerBlock.click();
  await expect(spoilerBlock).toHaveAttribute("data-revealed", "true");
  await expect(page.getByRole("dialog", { name: "image" })).toHaveCount(0);
});

test("spoiler button is disabled while attachment upload is pending", async ({
  page,
}) => {
  await installSpoilerBridge(page, { uploadDelayMs: 1_000 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const spoilerButton = page.getByRole("button", {
    name: "Spoiler",
    exact: true,
  });
  await expect(spoilerButton).toBeEnabled();

  await page.getByRole("button", { name: "Attach image" }).click();

  await expect(spoilerButton).toBeDisabled({ timeout: 500 });
  await expect(
    page.getByTestId("message-composer").getByAltText("Attachment cccc"),
  ).toBeVisible();
  await expect(spoilerButton).toBeEnabled();
});

test("hidden spoiler links reveal without opening on the first click", async ({
  page,
}) => {
  await installSpoilerBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("||[secret](https://example.com)||");
  await page.getByTestId("send-message").click();

  const lastMessage = page.getByTestId("message-row").last();
  const spoiler = lastMessage.locator(".buzz-spoiler").first();
  await expect(spoiler).toHaveAttribute("data-revealed", "false");

  const popupPromise = page
    .waitForEvent("popup", { timeout: 500 })
    .catch(() => null);
  await spoiler.getByRole("link", { name: "secret" }).click({ force: true });

  const popup = await popupPromise;
  await popup?.close();
  expect(popup).toBeNull();
  await expect(spoiler).toHaveAttribute("data-revealed", "true");
});

test("hidden spoilers stay masked on hover and focus until reveal", async ({
  page,
}) => {
  await installSpoilerBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("||secret hover focus note||");
  await page.getByTestId("send-message").click();

  const lastMessage = page.getByTestId("message-row").last();
  const spoiler = lastMessage.locator(".buzz-spoiler").first();
  const content = spoiler.locator(".buzz-spoiler__content");
  const particles = spoiler.locator(".buzz-spoiler__particles");

  await expect(spoiler).toHaveAttribute("data-revealed", "false");
  await expect(content).toHaveCSS("opacity", "0");
  await expect(particles).toHaveCSS("opacity", "1");

  await spoiler.hover();
  await expect(spoiler).toHaveAttribute("data-revealed", "false");
  await expect(content).toHaveCSS("opacity", "0");
  await expect(particles).toHaveCSS("opacity", "1");

  await page.mouse.move(0, 0);
  await spoiler.focus();
  await expect(spoiler).toBeFocused();
  await expect(spoiler).toHaveAttribute("data-revealed", "false");
  await expect(content).toHaveCSS("opacity", "0");
  await expect(particles).toHaveCSS("opacity", "1");

  await page.keyboard.press("Enter");
  await expect(spoiler).toHaveAttribute("data-revealed", "true");
  await expect(content).toHaveCSS("opacity", "1");
  await expect(particles).toHaveCSS("opacity", "0");
});

test("non-interactive inbox preview spoilers let row clicks pass through", async ({
  page,
}) => {
  await installSpoilerBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await page.waitForFunction(
    () =>
      typeof (window as MockFeedWindow).__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ ===
      "function",
  );

  await page.evaluate(
    ({ channelId, createdAt, currentPubkey, senderPubkey }) => {
      const pushFeedItem = (window as MockFeedWindow)
        .__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!pushFeedItem) {
        throw new Error("Mock feed injection helper is not installed.");
      }

      pushFeedItem({
        id: "mock-feed-spoiler-preview",
        kind: 9,
        pubkey: senderPubkey,
        content: "Preview contains ||hidden launch note|| for review.",
        created_at: createdAt,
        channel_id: channelId,
        channel_name: "general",
        tags: [
          ["e", channelId],
          ["p", currentPubkey],
        ],
        category: "mention",
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      createdAt: Math.floor(Date.now() / 1000),
      currentPubkey: TEST_IDENTITIES.tyler.pubkey,
      senderPubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );

  const item = page.getByTestId("home-inbox-item-mock-feed-spoiler-preview");
  await expect(item).toContainText("Preview contains");

  const spoiler = item.locator(".buzz-spoiler").first();
  await expect(spoiler).toBeVisible();
  await expect(spoiler).not.toHaveAttribute("role", "button");
  await expect(spoiler).not.toHaveAttribute("tabindex", "0");
  await expect(spoiler).toHaveCSS("pointer-events", "none");

  const box = await spoiler.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    throw new Error("Expected inbox preview spoiler to have a bounding box.");
  }

  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.getByTestId("home-inbox-detail")).toContainText(
    "Preview contains",
  );
});
