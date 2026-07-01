import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SIDEBAR_WIDTH_STORAGE_KEY = "buzz-sidebar-width";
const DEFAULT_SIDEBAR_WIDTH = 300;
const MOCK_PUBKEY = "deadbeef".repeat(8);
const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
const PRIORITY_SECTION = {
  id: "sec-priority",
  name: "Priority",
  icon: "\u2728",
  order: 0,
};

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function sidebarWidth(page: Page) {
  return page.getByTestId("app-sidebar").evaluate((element) => {
    return Math.round(element.getBoundingClientRect().width);
  });
}

async function storedSidebarWidth(page: Page) {
  return page.evaluate(
    (key) => localStorage.getItem(key),
    SIDEBAR_WIDTH_STORAGE_KEY,
  );
}

async function dragSidebarRail(page: Page, deltaX: number) {
  const sidebarRail = page.locator('[data-sidebar="rail"]');
  await expect(sidebarRail).toBeVisible();
  await expect(sidebarRail).toBeEnabled();

  const box = await sidebarRail.boundingBox();
  expect(box).not.toBeNull();

  if (!box) return;

  const startX = box.x + box.width / 2;
  const startY = box.y + box.height / 2;

  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX + deltaX, startY, { steps: 8 });
  await page.mouse.up();
}

async function seedChannelSections(
  page: Page,
  assignments: Record<string, string> = {},
) {
  await page.addInitScript(
    ({ pubkey, section, seededAssignments }) => {
      window.localStorage.setItem(
        `buzz-channel-sections.v1:${pubkey}`,
        JSON.stringify({
          version: 1,
          sections: [section],
          assignments: seededAssignments,
        }),
      );
    },
    {
      pubkey: MOCK_PUBKEY,
      section: PRIORITY_SECTION,
      seededAssignments: assignments,
    },
  );
}

async function visibleContextMenuRows(menu: Locator) {
  return menu.locator('[role="menuitem"]').evaluateAll((elements) => {
    return elements
      .filter((element): element is HTMLElement => {
        if (!(element instanceof HTMLElement)) return false;
        const rect = element.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0;
      })
      .map((element) => {
        const iconSlot = element.querySelector<HTMLElement>(
          "[data-sidebar-context-icon-slot]",
        );
        const label = Array.from(element.querySelectorAll("span")).find(
          (span) =>
            !span.hasAttribute("data-sidebar-context-icon-slot") &&
            span.textContent?.trim(),
        );

        return {
          iconLeft: iconSlot?.getBoundingClientRect().left ?? null,
          text: element.textContent?.trim() ?? "",
          textLeft: label?.getBoundingClientRect().left ?? null,
        };
      });
  });
}

async function expectMenuTextAligned(menu: Locator) {
  const rows = await visibleContextMenuRows(menu);
  const textLefts = rows
    .map((row) => row.textLeft)
    .filter((left): left is number => typeof left === "number");
  expect(textLefts.length).toBeGreaterThan(1);
  const firstTextLeft = textLefts[0];
  for (const textLeft of textLefts) {
    expect(Math.abs(textLeft - firstTextLeft)).toBeLessThanOrEqual(1);
  }
  for (const row of rows) {
    expect(row.iconLeft, row.text).not.toBeNull();
  }
}

async function expectAppClickable(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(() => getComputedStyle(document.body).pointerEvents),
    )
    .not.toBe("none");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function emojiPickerScrollTop(emojiPicker: Locator) {
  return emojiPicker.evaluate((element) => {
    const scroll = element.shadowRoot?.querySelector<HTMLElement>(".scroll");
    return scroll?.scrollTop ?? null;
  });
}

test("aligns custom section headers with the built-in sidebar sections", async ({
  page,
}) => {
  await seedChannelSections(page, {
    [RANDOM_CHANNEL_ID]: PRIORITY_SECTION.id,
  });
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  const customSectionButton = page.locator(
    `[aria-controls="sidebar-section-${PRIORITY_SECTION.id}"]`,
  );
  await expect(customSectionButton).toBeVisible();
  await expect(customSectionButton).toContainText(PRIORITY_SECTION.name);
  await expect(customSectionButton).toContainText(PRIORITY_SECTION.icon);
  await expect(
    customSectionButton.locator(".lucide-grip-vertical"),
  ).toHaveCount(0);
  await expect(customSectionButton.locator(".lucide-chevron-down")).toHaveCount(
    1,
  );

  const inlineGaps = await page.evaluate(
    ({ sectionId }) => {
      const sectionIcon = document.querySelector<HTMLElement>(
        `[data-testid="section-icon-${sectionId}"]`,
      );
      const sectionTitle = document.querySelector<HTMLElement>(
        `[data-testid="section-title-${sectionId}"]`,
      );
      const channelRow = document.querySelector<HTMLElement>(
        '[data-testid="channel-random"]',
      );
      const channelIcon = channelRow?.querySelector<HTMLElement>("svg");
      const channelTitle = Array.from(
        channelRow?.querySelectorAll<HTMLElement>("span") ?? [],
      ).find((span) => span.textContent?.trim() === "random");

      if (!sectionIcon || !sectionTitle || !channelIcon || !channelTitle) {
        throw new Error("Expected section and channel inline elements");
      }

      return {
        channel: Math.round(
          channelTitle.getBoundingClientRect().left -
            channelIcon.getBoundingClientRect().right,
        ),
        section: Math.round(
          sectionTitle.getBoundingClientRect().left -
            sectionIcon.getBoundingClientRect().right,
        ),
      };
    },
    { sectionId: PRIORITY_SECTION.id },
  );

  expect(inlineGaps.section).toBe(inlineGaps.channel);

  const sectionActions = page.getByTestId(
    `section-actions-${PRIORITY_SECTION.id}`,
  );
  await expect
    .poll(() =>
      sectionActions.evaluate((element) => getComputedStyle(element).opacity),
    )
    .toBe("0");

  await page.getByTestId("channel-random").hover();
  await expect
    .poll(() =>
      sectionActions.evaluate((element) => getComputedStyle(element).opacity),
    )
    .toBe("1");

  await page.mouse.move(500, 500);
  await expect
    .poll(() =>
      sectionActions.evaluate((element) => getComputedStyle(element).opacity),
    )
    .toBe("0");

  await customSectionButton.hover();
  await expect(sectionActions).toBeVisible();
  await expect(sectionActions.locator(".lucide-ellipsis-vertical")).toHaveCount(
    1,
  );
  await expect
    .poll(() =>
      sectionActions.evaluate((element) => getComputedStyle(element).opacity),
    )
    .toBe("1");
  await sectionActions.click();
  await expect(
    page.getByRole("menuitem", { name: "Rename section" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      sectionActions.evaluate((element) => getComputedStyle(element).opacity),
    )
    .toBe("1");
  await expect(page.getByRole("menuitem", { name: "Move up" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Move down" })).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Delete section" }),
  ).toBeVisible();
  await page.mouse.click(500, 500);
  await expect
    .poll(() =>
      sectionActions.evaluate((element) => getComputedStyle(element).opacity),
    )
    .toBe("0");
});

test("custom section emoji picker scrolls", async ({ page }) => {
  await seedChannelSections(page);
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page
    .locator(`[aria-controls="sidebar-section-${PRIORITY_SECTION.id}"]`)
    .hover();
  await page.getByTestId(`section-actions-${PRIORITY_SECTION.id}`).hover();
  await page.getByTestId(`section-actions-${PRIORITY_SECTION.id}`).click();
  await page.getByRole("menuitem", { name: "Rename section" }).click();

  await expect(
    page.getByRole("dialog", { name: "Rename section" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Choose section icon" }).click();
  const emojiPicker = page.locator("em-emoji-picker").first();
  await expect(emojiPicker).toBeVisible();

  await expect.poll(() => emojiPickerScrollTop(emojiPicker)).not.toBeNull();
  const beforeScrollTop = await emojiPickerScrollTop(emojiPicker);
  expect(beforeScrollTop).not.toBeNull();

  await emojiPicker.hover();
  await page.mouse.wheel(0, 700);

  await expect
    .poll(() => emojiPickerScrollTop(emojiPicker))
    .toBeGreaterThan(beforeScrollTop ?? 0);
});

test("orders and aligns channel and direct-message context menu items", async ({
  page,
}) => {
  await seedChannelSections(page);
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByTestId("channel-random").click({ button: "right" });
  await expect(
    page.getByRole("menuitem", { name: "Mark unread" }),
  ).toBeVisible();
  const channelMenu = page.getByRole("menu").first();
  await expectMenuTextAligned(channelMenu);

  const channelRows = await visibleContextMenuRows(channelMenu);
  expect(channelRows.map((row) => row.text)).toEqual([
    "Copy",
    "Move to section",
    "Mark unread",
    "Mute channel",
    "Star channel",
    "Leave channel",
  ]);

  const channelMenuBox = await channelMenu.boundingBox();
  expect(channelMenuBox?.width ?? 0).toBeGreaterThanOrEqual(238);

  await page.getByRole("menuitem", { name: "Copy" }).hover();
  await expect(
    page.getByRole("menuitem", { name: "Copy channel name" }),
  ).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Copy channel ID" }),
  ).toBeVisible();

  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await page.getByTestId("channel-alice-tyler").click({ button: "right" });
  await expect(page.getByRole("menuitem", { name: "Copy" })).toBeVisible();
  const dmMenu = page.getByRole("menu").first();
  await expectMenuTextAligned(dmMenu);

  const dmRows = await visibleContextMenuRows(dmMenu);
  expect(dmRows.map((row) => row.text)).toEqual([
    "Copy",
    "Mark unread",
    "Mute channel",
  ]);
});

test("channel context menu move and leave actions do not freeze the app", async ({
  page,
}) => {
  await seedChannelSections(page);
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Move to section" }).hover();
  await page.getByRole("menuitem", { name: PRIORITY_SECTION.name }).click();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        ({ pubkey, channelId }) => {
          const raw = window.localStorage.getItem(
            `buzz-channel-sections.v1:${pubkey}`,
          );
          const parsed = raw ? JSON.parse(raw) : null;
          return parsed?.assignments?.[channelId] ?? null;
        },
        { pubkey: MOCK_PUBKEY, channelId: RANDOM_CHANNEL_ID },
      ),
    )
    .toBe(PRIORITY_SECTION.id);
  await expectAppClickable(page);

  await page.getByTestId("channel-random").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Leave channel" }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expectAppClickable(page);
});

test("resizes, persists, and snaps to the default sidebar width", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await expect.poll(() => sidebarWidth(page)).toBe(DEFAULT_SIDEBAR_WIDTH);
  await expect.poll(() => storedSidebarWidth(page)).toBeNull();

  await dragSidebarRail(page, 64);

  await expect.poll(() => sidebarWidth(page)).toBe(364);
  await expect.poll(() => storedSidebarWidth(page)).toBe("364");

  await page.reload();
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect.poll(() => sidebarWidth(page)).toBe(364);

  await dragSidebarRail(page, -60);

  await expect.poll(() => sidebarWidth(page)).toBe(DEFAULT_SIDEBAR_WIDTH);
  await expect
    .poll(() => storedSidebarWidth(page))
    .toBe(String(DEFAULT_SIDEBAR_WIDTH));
});

test("shows a sidebar update card when an update is ready", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: { updateAvailable?: boolean } };
    };

    testWindow.__BUZZ_E2E__ = {
      ...(testWindow.__BUZZ_E2E__ ?? {}),
      mock: {
        ...(testWindow.__BUZZ_E2E__?.mock ?? {}),
        restartDelayMs: 500,
        updateAvailable: true,
      },
    };
  });

  await page.getByTestId("sidebar-profile-card").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-updates").click();
  await page.getByRole("button", { name: "Check for Updates" }).click();
  await expect(page.getByTestId("settings-panel-updates")).toContainText(
    "Update installed. Restart to apply.",
  );

  await page.getByTestId("settings-back-to-app").click();

  const updateCard = page.getByTestId("sidebar-update-card");
  await expect(updateCard).toBeVisible();
  await expect(updateCard).toContainText("Ready to update!");
  await expect(updateCard).toContainText("Click to restart");
  await expect(page.getByTestId("sidebar-update-restart")).toBeVisible();
  const reservedCardHeight = await updateCard.evaluate(
    (element) => (element as HTMLElement).offsetHeight,
  );

  await page.getByTestId("sidebar-update-restart").click();
  await expect(updateCard).toContainText("Restarting");
  await expect(page.getByTestId("sidebar-update-restart")).toBeDisabled();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMANDS__?: string[];
            }
          ).__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("plugin:process|restart");

  const dismissButton = page.getByTestId("sidebar-update-dismiss");
  await updateCard.hover();
  const dismissButtonBox = await dismissButton.boundingBox();
  expect(dismissButtonBox).not.toBeNull();
  if (!dismissButtonBox) return;

  await page.mouse.move(
    dismissButtonBox.x + dismissButtonBox.width / 2,
    dismissButtonBox.y + dismissButtonBox.height / 2,
  );
  await page.mouse.down();
  await expect(page.locator(".buzz-poof-burst")).toHaveCount(1);
  await expect(updateCard).toBeVisible();
  await page.mouse.up();
  await expect(updateCard).toHaveAttribute("data-dismissing", "true");
  await expect
    .poll(() =>
      updateCard.evaluate((element) => (element as HTMLElement).offsetHeight),
    )
    .toBe(reservedCardHeight);
  await expect
    .poll(() =>
      updateCard.evaluate((element) =>
        Number.parseFloat(getComputedStyle(element).opacity),
      ),
    )
    .toBeLessThan(0.05);
  await expect(updateCard).toBeHidden();
});
