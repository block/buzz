import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const OUTDIR = "test-results/hosted-communities";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: { pubkey_hex: DEFAULT_MOCK_PUBKEY },
    builderlabCommunities: [
      {
        id: "active-community",
        name: "E2E Test",
        normalized_host: "localhost:3000",
      },
      {
        id: "other-community",
        name: "Design studio",
        normalized_host: "design-studio.communities.buzz.xyz",
      },
    ],
  });
  await page.goto("/");
  await openSettings(page, "hosted-communities");
});

test("capture: community icon picker sits beside its hosted community", async ({
  page,
}) => {
  const activeRow = page
    .getByTestId("hosted-community-row")
    .filter({ hasText: "E2E Test" });
  const otherRow = page
    .getByTestId("hosted-community-row")
    .filter({ hasText: "Design studio" });

  await expect(activeRow.getByTestId("community-icon-settings")).toBeVisible();
  await expect(otherRow.getByTestId("community-icon-settings")).toHaveCount(0);

  const iconDataUrl = `data:image/svg+xml,${encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" rx="28" fill="#ff56c3"/><text x="64" y="80" text-anchor="middle" font-size="48">😅</text></svg>',
  )}`;
  await activeRow.getByLabel("Add community icon").click();

  const picker = page.getByRole("group", { name: "Community icon picker" });
  await expect(picker).toBeVisible();
  await expect(page.getByRole("tab", { name: "Image" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Emoji" })).toBeVisible();
  await page.getByPlaceholder("Paste a URL").fill(iconDataUrl);
  await page.getByRole("button", { name: "Apply" }).click();

  const icon = activeRow.getByRole("img", { name: /community icon$/i });
  await expect(icon).toBeVisible();
  const maskImage = await activeRow
    .getByTestId("community-icon-mask")
    .evaluate((element) => getComputedStyle(element).webkitMaskImage);
  expect(maskImage).toContain("radial-gradient");
  await expect(page.getByTestId("community-icon-save")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_SIGNED_EVENTS__?.some(
          (event) =>
            event.kind === 9033 &&
            event.tags.some(
              (tag) => tag[0] === "icon" && tag[1]?.startsWith("data:image/"),
            ),
        ),
      ),
    )
    .toBe(true);

  const iconBox = await activeRow
    .getByTestId("community-icon-settings")
    .boundingBox();
  const nameBox = await activeRow
    .getByText("E2E Test", { exact: true })
    .boundingBox();
  expect(iconBox).not.toBeNull();
  expect(nameBox).not.toBeNull();
  expect(iconBox?.x ?? Number.POSITIVE_INFINITY).toBeLessThan(
    nameBox?.x ?? Number.NEGATIVE_INFINITY,
  );

  await waitForAnimations(page);
  await activeRow.screenshot({
    path: `${OUTDIR}/01-community-icon-row.png`,
  });
});
