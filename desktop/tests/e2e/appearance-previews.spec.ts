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

test("workspace preview updates locally and commits only on selection", async ({
  page,
}) => {
  await openAppearance(page);

  const workspace = page.getByTestId("appearance-workspace-preview-surface");
  const linkSample = page.getByTestId("appearance-workspace-link-preview");
  const preview = page.getByTestId("appearance-workspace-preview");
  await expect(preview.getByText("Preview", { exact: true })).toBeVisible();
  await expect(preview.getByText("Try the channel and thread")).toHaveCount(0);
  await expect(preview.getByText("Settings", { exact: true })).toHaveCount(0);
  await expect(workspace).toHaveCSS("border-radius", "12px");
  await expect(workspace).toHaveCSS("box-shadow", "none");
  await expect(workspace).toHaveAttribute("data-link-style", "compact");
  await expect(workspace).toHaveAttribute("data-thread-mode", "split");
  await expect(linkSample).toHaveAttribute("inert", "");
  await expect(linkSample.locator("[data-link-preview-inline]")).toHaveCount(0);

  const linkControl = page.getByTestId("link-preview-style-control");
  const richOption = page.getByTestId("link-preview-style-rich");
  await scrubTo(linkControl, richOption);
  await expect(
    page.getByTestId("appearance-workspace-preview-thread"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("appearance-preview-open-thread"),
  ).toBeVisible();
  await expect(workspace).toHaveAttribute("data-link-style", "rich");
  await expect(
    page.getByTestId("appearance-workspace-preview-channel").last(),
  ).toHaveAttribute("data-motion-mode", "direct");
  await expect(linkSample.locator("[data-link-preview-inline]")).toBeVisible();
  await expect(linkSample.getByText("Show less")).toHaveCount(0);
  await expect(
    page.getByText("Large previews with images and descriptions"),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        LINK_PREVIEW_STYLE_STORAGE_KEY,
      ),
    )
    .toBe("compact");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(workspace).toHaveAttribute("data-link-style", "compact");
  await expect(linkSample.locator("[data-link-preview-inline]")).toHaveCount(0);
  await expect(page.getByTestId("link-preview-style-compact")).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  await richOption.click();
  await expect(workspace).toHaveAttribute("data-link-style", "rich");
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
  await scrubTo(threadControl, focusOption);
  await expect(
    page.getByTestId("appearance-workspace-preview-thread").last(),
  ).toBeVisible();
  await expect(workspace).toHaveAttribute("data-thread-mode", "focus");
  const directThread = page
    .getByTestId("appearance-workspace-preview-thread")
    .last();
  await expect(directThread).toHaveAttribute("data-motion-mode", "direct");
  expect(await directThread.evaluate((element) => element.style.width)).toBe(
    "",
  );
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
  await expect(workspace).toHaveAttribute("data-thread-mode", "split");

  await focusOption.click();
  await expect(workspace).toHaveAttribute("data-thread-mode", "focus");
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        THREAD_VIEW_MODE_STORAGE_KEY,
      ),
    )
    .toBe("focus");

  await page.getByTestId("appearance-preview-close-thread").last().click();
  await expect(
    page.getByTestId("appearance-workspace-preview-thread"),
  ).toHaveCount(0);
  await page.getByTestId("appearance-preview-open-thread").click();
  await expect(
    page.getByTestId("appearance-workspace-preview-thread"),
  ).toBeVisible();
});

test("appearance controls keep polish aligned with reduced motion", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openAppearance(page);

  const thread = page.getByTestId("appearance-workspace-preview-thread");
  await expect(thread).toHaveAttribute("data-motion-mode", "reduced");
  await expect(thread).toHaveCSS("filter", "blur(0px)");

  const accent = page.getByTestId("accent-color-blue");
  await accent.hover();
  await expect(accent).toHaveCSS("transform", "none");
  await expect(accent).toHaveCSS("border-top-width", "0px");

  const selectedAccentDot = page.getByTestId("accent-color-selection");
  await expect(selectedAccentDot).toHaveCSS("width", "8px");
  await expect(selectedAccentDot).toHaveCSS("height", "8px");
  await expect(selectedAccentDot).toHaveCSS(
    "background-color",
    "rgb(255, 255, 255)",
  );
  await expect(selectedAccentDot).toHaveCSS("opacity", "1");

  const accentRow = page.getByTestId("accent-color-row");
  const accentLabel = accentRow.getByText("Accent color", { exact: true });
  const accentOptions = page.getByTestId("accent-color-options");
  const [labelBox, optionsBox] = await Promise.all([
    accentLabel.boundingBox(),
    accentOptions.boundingBox(),
  ]);
  if (!labelBox || !optionsBox) throw new Error("Accent geometry is missing");
  expect(
    Math.abs(
      labelBox.y + labelBox.height / 2 - (optionsBox.y + optionsBox.height / 2),
    ),
  ).toBeLessThanOrEqual(1);

  await expect(page.getByTestId("conversation-density-row")).toHaveCSS(
    "border-top-width",
    "1px",
  );
});

test("workspace preview stays responsive with stacked controls", async ({
  page,
}) => {
  await page.setViewportSize({ width: 840, height: 900 });
  await openAppearance(page, {
    linkStyle: "rich",
    theme: "buzz-dark",
    threadMode: "focus",
  });

  const preferencesCard = page.getByTestId("appearance-preferences-card");
  const workspace = page.getByTestId("appearance-workspace-preview-surface");
  const linkControl = page.getByTestId("link-preview-style-control");
  const threadControl = page.getByTestId("thread-layout-control");

  await expect(workspace).toBeVisible();
  await expect(workspace).toHaveAttribute("data-link-style", "rich");
  await expect(workspace).toHaveAttribute("data-thread-mode", "focus");

  const [cardBox, workspaceBox, linkControlBox, threadControlBox] =
    await Promise.all([
      preferencesCard.boundingBox(),
      workspace.boundingBox(),
      linkControl.boundingBox(),
      threadControl.boundingBox(),
    ]);
  if (!cardBox || !workspaceBox || !linkControlBox || !threadControlBox) {
    throw new Error("Responsive Appearance geometry is missing");
  }
  expect(workspaceBox.width).toBeLessThanOrEqual(640);
  expect(linkControlBox.x + linkControlBox.width).toBeLessThanOrEqual(
    cardBox.x + cardBox.width,
  );
  expect(threadControlBox.x + threadControlBox.width).toBeLessThanOrEqual(
    cardBox.x + cardBox.width,
  );

  await waitForAnimations(page);
  await page.getByTestId("appearance-workspace-preview").screenshot({
    path: `${SHOTS}/01-workspace-rich-focus-dark-narrow.png`,
  });
});

test("workspace preview renders compact and split at wide width", async ({
  page,
}) => {
  await openAppearance(page);
  await waitForAnimations(page);
  await page.getByTestId("appearance-workspace-preview").screenshot({
    path: `${SHOTS}/02-workspace-compact-split-light-wide.png`,
  });
});
