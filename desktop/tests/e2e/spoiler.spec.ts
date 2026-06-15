import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

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
