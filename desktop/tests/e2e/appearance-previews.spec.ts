import { expect, test, type Locator, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/appearance-previews";
const THEME_STORAGE_KEY = "buzz-theme";
const LINK_PREVIEW_STYLE_STORAGE_KEY = "buzz.appearance.linkPreviewStyle";
const THREAD_VIEW_MODE_STORAGE_KEY = "buzz.channels.threadViewMode";

async function openAppearance(
  page: Page,
  {
    linkStyle = "compact",
    theme = "buzz",
    threadMode = "split",
  }: {
    linkStyle?: "compact" | "rich";
    theme?: "buzz" | "buzz-dark";
    threadMode?: "focus" | "split";
  } = {},
) {
  await page.addInitScript(
    ({ linkKey, linkStyle, theme, themeKey, threadKey, threadMode }) => {
      window.localStorage.setItem(themeKey, theme);
      window.localStorage.setItem(linkKey, linkStyle);
      window.localStorage.setItem(threadKey, threadMode);
    },
    {
      linkKey: LINK_PREVIEW_STYLE_STORAGE_KEY,
      linkStyle,
      theme,
      themeKey: THEME_STORAGE_KEY,
      threadKey: THREAD_VIEW_MODE_STORAGE_KEY,
      threadMode,
    },
  );
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-appearance").click();
  await expect(page.getByTestId("settings-theme")).toBeVisible({
    timeout: 10_000,
  });
  await waitForAnimations(page);
}

async function scrubTo(control: Locator, option: Locator) {
  await control.scrollIntoViewIfNeeded();
  const controlBox = await control.boundingBox();
  const optionBox = await option.boundingBox();
  if (!controlBox || !optionBox) throw new Error("Segment geometry is missing");

  const selectedOption = control.locator('button[aria-pressed="true"]');
  const selectedOptionBox = await selectedOption.boundingBox();
  if (!selectedOptionBox)
    throw new Error("Starting segment geometry is missing");

  await control
    .page()
    .mouse.move(
      selectedOptionBox.x + selectedOptionBox.width / 2,
      controlBox.y + controlBox.height / 2,
    );
  await control.page().mouse.down();
  await control
    .page()
    .mouse.move(
      optionBox.x + optionBox.width / 2,
      controlBox.y + controlBox.height / 2,
      { steps: 5 },
    );
}

test("appearance samples preview locally and commit only on selection", async ({
  page,
}) => {
  await openAppearance(page);

  const linkControl = page.getByTestId("link-preview-style-control");
  const richOption = page.getByTestId("link-preview-style-rich");
  const linkSample = page.getByTestId("link-preview-sample");
  await expect(linkSample.locator("[data-link-preview-inline]")).toHaveCount(0);
  await expect(page.getByTestId("link-preview-sample-surface")).toHaveAttribute(
    "inert",
    "",
  );
  const sampleLink = linkSample.locator("a").first();
  await sampleLink.evaluate((element) => element.focus());
  await expect(sampleLink).not.toBeFocused();
  await scrubTo(linkControl, richOption);
  await expect(linkSample.locator("[data-link-preview-inline]")).toBeVisible();
  await expect(linkSample.getByText("Show less")).toHaveCount(0);
  await expect(
    page.getByText("Large previews with images and descriptions"),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        LINK_PREVIEW_STYLE_STORAGE_KEY,
      ),
    )
    .toBe("compact");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(linkSample.locator("[data-link-preview-inline]")).toHaveCount(0);
  await expect(page.getByTestId("link-preview-style-compact")).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  await richOption.click();
  await expect(linkSample.locator("[data-link-preview-inline]")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        LINK_PREVIEW_STYLE_STORAGE_KEY,
      ),
    )
    .toBe("rich");

  const threadControl = page.getByTestId("thread-layout-control");
  const focusOption = page.getByTestId("thread-layout-focus");
  await expect(page.getByTestId("thread-layout-diagram-split")).toBeVisible();
  await scrubTo(threadControl, focusOption);
  await expect(page.getByTestId("thread-layout-diagram-focus")).toBeVisible();
  await expect(page.getByText("Threads open over the channel")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        THREAD_VIEW_MODE_STORAGE_KEY,
      ),
    )
    .toBe("split");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(page.getByTestId("thread-layout-diagram-split")).toBeVisible();

  await focusOption.click();
  await expect(page.getByTestId("thread-layout-diagram-focus")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        THREAD_VIEW_MODE_STORAGE_KEY,
      ),
    )
    .toBe("focus");
});

test("appearance previews stay grouped and responsive", async ({ page }) => {
  await page.setViewportSize({ width: 840, height: 900 });
  await openAppearance(page, {
    linkStyle: "rich",
    theme: "buzz-dark",
    threadMode: "focus",
  });

  const preferencesCard = page.getByTestId("appearance-preferences-card");
  const linkGroup = page.getByTestId("link-preview-style-group");
  const threadGroup = page.getByTestId("thread-layout-group");
  const linkControl = page.getByTestId("link-preview-style-control");
  const threadControl = page.getByTestId("thread-layout-control");

  await expect(linkGroup.getByText("Preview", { exact: true })).toBeVisible();
  await expect(threadGroup.getByText("Preview", { exact: true })).toBeVisible();

  const [cardBox, linkControlBox, threadControlBox] = await Promise.all([
    preferencesCard.boundingBox(),
    linkControl.boundingBox(),
    threadControl.boundingBox(),
  ]);
  if (!cardBox || !linkControlBox || !threadControlBox) {
    throw new Error("Responsive Appearance geometry is missing");
  }
  expect(linkControlBox.width).toBeGreaterThan(cardBox.width - 40);
  expect(threadControlBox.width).toBeGreaterThan(cardBox.width - 40);

  await waitForAnimations(page);
  await linkGroup.screenshot({
    path: `${SHOTS}/01-link-preview-rich-dark-narrow.png`,
  });
  await threadGroup.screenshot({
    path: `${SHOTS}/02-thread-focus-dark-narrow.png`,
  });
});

test("appearance previews render compact and split samples at wide width", async ({
  page,
}) => {
  await openAppearance(page);
  await waitForAnimations(page);
  await page.getByTestId("link-preview-style-group").screenshot({
    path: `${SHOTS}/03-link-preview-compact-light-wide.png`,
  });
  await page.getByTestId("thread-layout-group").screenshot({
    path: `${SHOTS}/04-thread-split-light-wide.png`,
  });
});
