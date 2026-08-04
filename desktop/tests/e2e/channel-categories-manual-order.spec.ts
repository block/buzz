import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const MOCK_PUBKEY = "deadbeef".repeat(8);
const RELAY = encodeURIComponent("ws://localhost:3000");
const SECTION_KEY = `buzz-channel-sections.v1:${MOCK_PUBKEY}:${RELAY}`;
const SORT_KEY = `buzz-channel-sort.v1:${MOCK_PUBKEY}:${RELAY}`;
const ORDER_KEY = `buzz-channel-manual-order.v1:${MOCK_PUBKEY}:${RELAY}`;

async function openApp(page: Page) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function openSort(page: Page, actionsTestId: string) {
  await page.getByTestId(actionsTestId).click();
  await page.getByRole("menuitem", { name: "Sort" }).click();
}

async function dragOver(
  page: Page,
  source: Locator,
  target: Locator,
  settleStaticTarget = false,
) {
  const from = await source.boundingBox();
  if (!from) throw new Error("drag source not laid out");
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 + 10);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
  const to = await target.boundingBox();
  if (!to) throw new Error("drag target not laid out");
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, {
    steps: 10,
  });
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
  if (settleStaticTarget) {
    const settledTarget = await target.boundingBox();
    if (!settledTarget) throw new Error("drag target disappeared");
    await page.mouse.move(
      settledTarget.x + settledTarget.width / 2,
      settledTarget.y + settledTarget.height / 2,
    );
  }
  await expect(page.getByRole("status")).toContainText(" is over ");
  await page.mouse.up();
}

async function waitForMenusToClose(page: Page) {
  await expect(page.getByRole("menu")).toHaveCount(0);
}

async function waitForKeyboardSensor(page: Page, row: Locator) {
  await expect(row).toHaveClass(/opacity-30/);
  await page.evaluate(
    () => new Promise<void>((resolve) => window.setTimeout(resolve, 0)),
  );
}

async function keyboardMoveUp(
  page: Page,
  source: Locator,
  row: Locator,
  channelName: string,
) {
  await source.focus();
  await source.press("Space");
  await waitForKeyboardSensor(page, row);
  await source.press("ArrowUp");
  await expect(page.getByRole("status")).toContainText(
    `Channel ${channelName} is over position 1 in category Channels.`,
  );
  await source.press("Space");
}

function draggableChannel(page: Page, name: string) {
  return page.locator("[data-dnd-channel]").filter({
    has: page.getByTestId(`channel-${name}`),
  });
}

function channelDragHandle(page: Page, name: string) {
  return draggableChannel(page, name).locator("[data-dnd-handle]");
}

function channelNames(list: Locator) {
  return list.locator("[data-testid^='channel-']").evaluateAll((nodes) =>
    nodes
      .map((node) => node.getAttribute("data-testid") ?? "")
      .filter(
        (id) =>
          !id.startsWith("channel-unread") &&
          !id.startsWith("channel-working") &&
          !id.startsWith("channel-dm-count"),
      )
      .map((id) => id.replace(/^channel-/, "")),
  );
}

test.describe("sidebar categories and manual channel order", () => {
  test("creates a category from the Channels menu and rejects a duplicate", async ({
    page,
  }) => {
    await openApp(page);

    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "New category..." }).click();
    await page.getByPlaceholder("Category name").fill("Work");
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page.getByText("Work", { exact: true })).toBeVisible();

    const stored = await page.evaluate((key) => {
      return JSON.parse(window.localStorage.getItem(key) ?? "null");
    }, SECTION_KEY);
    expect(stored.sections).toHaveLength(1);
    expect(stored.sections[0].name).toBe("Work");

    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "New category..." }).click();
    await page.getByPlaceholder("Category name").fill("work");
    await expect(page.getByRole("button", { name: "Create" })).toBeDisabled();
    await expect(
      page.getByText("A category with this name already exists."),
    ).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("section-actions-channels")).toBeFocused();
  });

  test("renames and deletes a category from its actions menu", async ({
    page,
  }) => {
    await openApp(page);
    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "New category..." }).click();
    await page.getByPlaceholder("Category name").fill("Work");
    await page.getByRole("button", { name: "Create" }).click();

    const section = await page.evaluate((key) => {
      const value = JSON.parse(window.localStorage.getItem(key) ?? "null");
      return value.sections[0] as { id: string };
    }, SECTION_KEY);
    const actions = page.getByTestId(`section-actions-${section.id}`);
    await page.getByText("Work", { exact: true }).hover();
    await actions.click();
    await page.getByRole("menuitem", { name: "Rename category" }).click();
    await page.getByPlaceholder("Category name").fill("Projects");
    await page.getByRole("button", { name: "Save" }).click();
    const title = page.getByTestId(`section-title-${section.id}`);
    await expect(title).toHaveText("Projects");

    await title.hover();
    await actions.click();
    await page.getByRole("menuitem", { name: "Delete category" }).click();
    await page.getByRole("button", { name: "Delete" }).click();
    await expect(title).toHaveCount(0);
  });

  test("Manual reorders Channels and restores the exact order after reload", async ({
    page,
  }) => {
    await openApp(page);

    await page.getByText("Channels", { exact: true }).hover();
    await openSort(page, "section-actions-channels");
    await page.getByRole("menuitemradio", { name: "Manual" }).click();
    await waitForMenusToClose(page);

    const list = page.getByTestId("stream-list");
    const initialOrder = (await channelNames(list)).slice(0, 2);
    expect(initialOrder).toHaveLength(2);
    const [firstChannel, secondChannel] = initialOrder;
    const firstRow = draggableChannel(page, firstChannel);
    const firstHandle = channelDragHandle(page, firstChannel);
    const channelRows = list.locator("[data-dnd-channel]").locator("..");
    const manualRowVisibility = await channelRows.evaluateAll((rows) =>
      rows.map((row) => getComputedStyle(row).contentVisibility),
    );
    expect(manualRowVisibility).toEqual(
      Array(manualRowVisibility.length).fill("visible"),
    );
    await expect(firstRow).not.toHaveClass(/touch-none/);
    await expect(firstHandle).toHaveClass(/touch-none/);
    await page.mouse.move(0, 0);
    await expect(firstHandle).toHaveCSS("opacity", "0");
    await firstRow.hover();
    await expect(firstHandle).toHaveCSS("opacity", "1");
    const secondHandle = channelDragHandle(page, secondChannel);
    await draggableChannel(page, secondChannel).hover();
    await expect(firstHandle).toHaveCSS("opacity", "0");
    await expect(secondHandle).toHaveCSS("opacity", "1");
    await dragOver(page, firstHandle, draggableChannel(page, secondChannel));
    await expect
      .poll(async () => (await channelNames(list)).slice(0, 2))
      .toEqual([secondChannel, firstChannel]);
    await expect(page.getByRole("status")).toContainText(
      `Moved channel ${firstChannel} to position 2 in category Channels.`,
    );
    expect(
      await channelRows.evaluateAll((rows) =>
        rows.map((row) => getComputedStyle(row).contentVisibility),
      ),
    ).toEqual(Array(manualRowVisibility.length).fill("visible"));
    for (const channelName of await channelNames(list)) {
      await expect(page.getByTestId(`channel-${channelName}`)).toBeVisible();
    }

    const persisted = await page.evaluate(
      ({ sortKey, orderKey }) => ({
        sort: JSON.parse(window.localStorage.getItem(sortKey) ?? "null"),
        order: JSON.parse(window.localStorage.getItem(orderKey) ?? "null"),
      }),
      { sortKey: SORT_KEY, orderKey: ORDER_KEY },
    );
    expect(persisted.sort?.groups?.channels).toBeUndefined();
    expect(persisted.order.manualGroups).toContain("channels");
    expect(persisted.order.groups.channels).toHaveLength(
      (await channelNames(list)).length,
    );

    await page.reload();
    await expect
      .poll(async () =>
        (await channelNames(page.getByTestId("stream-list"))).slice(0, 2),
      )
      .toEqual([secondChannel, firstChannel]);
  });

  test("keyboard users can move a channel in Manual order", async ({
    page,
  }) => {
    await openApp(page);
    await page.getByText("Channels", { exact: true }).hover();
    await openSort(page, "section-actions-channels");
    await page.getByRole("menuitemradio", { name: "Manual" }).click();
    await waitForMenusToClose(page);

    const list = page.getByTestId("stream-list");
    const initialOrder = (await channelNames(list)).slice(0, 2);
    expect(initialOrder).toHaveLength(2);
    const [firstChannel, secondChannel] = initialOrder;
    const secondHandle = channelDragHandle(page, secondChannel);
    await page.mouse.move(0, 0);
    await expect(secondHandle).toHaveCSS("opacity", "0");
    await secondHandle.focus();
    await expect(secondHandle).toHaveCSS("opacity", "1");
    await secondHandle.press("Space");
    await waitForKeyboardSensor(page, draggableChannel(page, secondChannel));
    await expect(secondHandle).toHaveCSS("opacity", "1");
    // KeyboardSensor attaches its document keydown listener on the next task.
    // The helper yields a macrotask so Escape exercises the active drag instead
    // of racing that listener installation.
    await secondHandle.press("Escape");
    await expect(page.getByRole("status")).toContainText(
      `Cancelled moving channel ${secondChannel}.`,
    );
    expect((await channelNames(list)).slice(0, 2)).toEqual(initialOrder);

    await keyboardMoveUp(
      page,
      secondHandle,
      draggableChannel(page, secondChannel),
      secondChannel,
    );
    await expect
      .poll(async () => (await channelNames(list)).slice(0, 2))
      .toEqual([secondChannel, firstChannel]);
    await expect(page.getByRole("status")).toContainText(
      `Moved channel ${secondChannel} to position 1 in category Channels.`,
    );
  });

  test("moves channels into an empty category and inserts at a manual position", async ({
    page,
  }) => {
    await openApp(page);

    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "New category..." }).click();
    await page.getByPlaceholder("Category name").fill("Work");
    await page.getByRole("button", { name: "Create" }).click();

    const section = await page.evaluate((key) => {
      const value = JSON.parse(window.localStorage.getItem(key) ?? "null");
      return value.sections[0] as { id: string };
    }, SECTION_KEY);
    await openSort(page, `section-actions-${section.id}`);
    await page.getByRole("menuitemradio", { name: "Manual" }).click();
    await waitForMenusToClose(page);

    await dragOver(
      page,
      channelDragHandle(page, "agents"),
      page.getByTestId(`section-empty-${section.id}`),
      true,
    );
    const categoryList = page.getByTestId(`section-list-${section.id}`);
    await expect(categoryList.getByTestId("channel-agents")).toBeVisible();
    await expect(page.getByRole("status")).toContainText(
      "Moved channel agents to position 1 in category Work.",
    );
    // Let dnd-kit's drop transform finish before measuring the next drag
    // targets; otherwise the second pointer sequence can hit the stale overlay.
    await page.waitForTimeout(250);

    await dragOver(
      page,
      channelDragHandle(page, "general"),
      draggableChannel(page, "agents"),
    );
    await expect
      .poll(async () => channelNames(categoryList))
      .toEqual(["general", "agents"]);
    await expect(page.getByRole("status")).toContainText(
      "Moved channel general to position 1 in category Work.",
    );

    const stored = await page.evaluate(
      ({ sectionKey, orderKey }) => ({
        sections: JSON.parse(window.localStorage.getItem(sectionKey) ?? "null"),
        order: JSON.parse(window.localStorage.getItem(orderKey) ?? "null"),
      }),
      { sectionKey: SECTION_KEY, orderKey: ORDER_KEY },
    );
    expect(Object.keys(stored.sections.assignments)).toHaveLength(2);
    expect(stored.order.groups[`section:${section.id}`]).toHaveLength(2);
  });

  test("Move up/down reorders a category across Channels and persists after reload", async ({
    page,
  }) => {
    await openApp(page);

    // Create two categories so the movable lane is [Work, Home, Channels].
    for (const name of ["Work", "Home"]) {
      await page.getByText("Channels", { exact: true }).hover();
      await page.getByTestId("section-actions-channels").click();
      await page.getByRole("menuitem", { name: "New category..." }).click();
      await page.getByPlaceholder("Category name").fill(name);
      await page.getByRole("button", { name: "Create" }).click();
      await expect(page.getByText(name, { exact: true })).toBeVisible();
    }

    const sections = await page.evaluate((key) => {
      const value = JSON.parse(window.localStorage.getItem(key) ?? "null");
      return value.sections as { id: string; name: string }[];
    }, SECTION_KEY);
    const work = sections.find((s) => s.name === "Work");
    const home = sections.find((s) => s.name === "Home");
    expect(work && home).toBeTruthy();
    if (!work || !home) throw new Error("categories missing");

    // Move Work down past Home → [Home, Work, Channels]
    await page.getByTestId(`section-title-${work.id}`).hover();
    await page.getByTestId(`section-actions-${work.id}`).click();
    await page.getByRole("menuitem", { name: "Move down" }).click();
    await waitForMenusToClose(page);

    // Move Channels up past Work → [Home, Channels, Work]
    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "Move up" }).click();
    await waitForMenusToClose(page);

    const afterMove = await page.evaluate((key) => {
      return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
        channelsBlockIndex?: number;
        sections: { id: string; name: string; order: number }[];
      };
    }, SECTION_KEY);
    expect(afterMove.channelsBlockIndex).toBe(1);
    const ordered = afterMove.sections
      .slice()
      .sort((a, b) => a.order - b.order)
      .map((s) => s.name);
    expect(ordered).toEqual(["Home", "Work"]);

    // Delete Home (before Channels): must keep Channels above Work.
    await page.getByTestId(`section-title-${home.id}`).hover();
    await page.getByTestId(`section-actions-${home.id}`).click();
    await page.getByRole("menuitem", { name: "Delete category" }).click();
    await page.getByRole("button", { name: "Delete" }).click();
    await expect(page.getByTestId(`section-title-${home.id}`)).toHaveCount(0);

    const afterDelete = await page.evaluate((key) => {
      return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
        channelsBlockIndex?: number;
        sections: { id: string; name: string }[];
      };
    }, SECTION_KEY);
    expect(afterDelete.sections.map((s) => s.name)).toEqual(["Work"]);
    expect(afterDelete.channelsBlockIndex).toBe(0);

    await page.reload();
    await page.getByTestId("channel-general").click();
    const afterReload = await page.evaluate((key) => {
      return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
        channelsBlockIndex?: number;
        sections: { id: string; name: string }[];
      };
    }, SECTION_KEY);
    expect(afterReload.channelsBlockIndex).toBe(0);
    expect(afterReload.sections.map((s) => s.name)).toEqual(["Work"]);
  });

  test("category block drag handle reorders relative to Channels", async ({
    page,
  }) => {
    await openApp(page);
    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "New category..." }).click();
    await page.getByPlaceholder("Category name").fill("Work");
    await page.getByRole("button", { name: "Create" }).click();

    const section = await page.evaluate((key) => {
      const value = JSON.parse(window.localStorage.getItem(key) ?? "null");
      return value.sections[0] as { id: string };
    }, SECTION_KEY);

    const categoryHandle = page.getByTestId(`block-drag-${section.id}`);
    const channelsHandle = page.getByTestId("block-drag-__channels__");
    const categoryTitle = page.getByTestId(`section-title-${section.id}`);
    const channelsLabel = page.getByTestId("stream-list-section-label");
    const channelsTitle = channelsLabel.locator("[data-sidebar-section-title]");
    const categoryActions = page.getByTestId(`section-actions-${section.id}`);
    const channelsActions = page.getByTestId("section-actions-channels");

    await page.evaluate(() => {
      if (document.activeElement instanceof HTMLElement) {
        document.activeElement.blur();
      }
    });
    await page.mouse.move(0, 0);
    await expect(categoryHandle).toHaveCSS("opacity", "0");
    await expect(channelsHandle).toHaveCSS("opacity", "0");

    const categoryTitleBox = await categoryTitle.boundingBox();
    const channelsTitleBox = await channelsTitle.boundingBox();
    expect(categoryTitleBox).not.toBeNull();
    expect(channelsTitleBox).not.toBeNull();
    if (!categoryTitleBox || !channelsTitleBox) {
      throw new Error("category headers are not laid out");
    }
    expect(Math.abs(categoryTitleBox.x - channelsTitleBox.x)).toBeLessThan(1);

    await categoryTitle.hover();
    await expect(categoryHandle).toHaveCSS("opacity", "1");
    const categoryHandleBox = await categoryHandle.boundingBox();
    const categoryActionsBox = await categoryActions.boundingBox();
    expect(categoryHandleBox).not.toBeNull();
    expect(categoryActionsBox).not.toBeNull();
    if (!categoryHandleBox || !categoryActionsBox) {
      throw new Error("category actions are not laid out");
    }
    expect(categoryHandleBox.x).toBeGreaterThan(categoryActionsBox.x);

    await channelsLabel.hover();
    await expect(channelsHandle).toHaveCSS("opacity", "1");
    const channelsHandleBox = await channelsHandle.boundingBox();
    const channelsActionsBox = await channelsActions.boundingBox();
    expect(channelsHandleBox).not.toBeNull();
    expect(channelsActionsBox).not.toBeNull();
    if (!channelsHandleBox || !channelsActionsBox) {
      throw new Error("Channels actions are not laid out");
    }
    expect(channelsHandleBox.x).toBeGreaterThan(channelsActionsBox.x);

    await page.evaluate(() => {
      if (document.activeElement instanceof HTMLElement) {
        document.activeElement.blur();
      }
    });
    await page.mouse.move(0, 0);
    await expect(categoryHandle).toHaveCSS("opacity", "0");
    await expect(channelsHandle).toHaveCSS("opacity", "0");

    // Drag Work below Channels → [Channels, Work]
    await dragOver(page, categoryHandle, channelsHandle);
    await expect
      .poll(async () => {
        const stored = await page.evaluate((key) => {
          return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
            channelsBlockIndex?: number;
          };
        }, SECTION_KEY);
        return stored.channelsBlockIndex;
      })
      .toBe(0);

    await page.reload();
    await page.getByTestId("channel-general").click();
    await expect
      .poll(async () => {
        const stored = await page.evaluate((key) => {
          return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
            channelsBlockIndex?: number;
          };
        }, SECTION_KEY);
        return stored.channelsBlockIndex;
      })
      .toBe(0);
  });

  test("keyboard users can move a category block across Channels and persist after reload", async ({
    page,
  }) => {
    await openApp(page);
    await page.getByText("Channels", { exact: true }).hover();
    await page.getByTestId("section-actions-channels").click();
    await page.getByRole("menuitem", { name: "New category..." }).click();
    await page.getByPlaceholder("Category name").fill("Work");
    await page.getByRole("button", { name: "Create" }).click();

    const section = await page.evaluate((key) => {
      const value = JSON.parse(window.localStorage.getItem(key) ?? "null");
      return value.sections[0] as { id: string };
    }, SECTION_KEY);

    // Default layout: [Work, Channels]. Keyboard-move Work down past Channels.
    const categoryHandle = page.getByTestId(`block-drag-${section.id}`);
    const categoryShell = page.locator(`[data-dnd-block="${section.id}"]`);
    await page.mouse.move(0, 0);
    await expect(categoryHandle).toHaveCSS("opacity", "0");
    await categoryHandle.focus();
    await expect(categoryHandle).toHaveCSS("opacity", "1");

    await categoryHandle.press("Space");
    // opacity-30 is applied to the section body inside the shell, not the
    // data-dnd-block wrapper (unlike channel rows). Live-region text only
    // retains the latest announcement, so do not require "Picked up".
    await expect(categoryShell.locator(".opacity-30")).toBeVisible();
    // KeyboardSensor attaches its document keydown listener on the next task.
    await page.evaluate(
      () => new Promise<void>((resolve) => window.setTimeout(resolve, 0)),
    );
    await categoryHandle.press("ArrowDown");
    await expect(page.getByRole("status")).toContainText("is over Channels");
    await categoryHandle.press("Space");
    await expect(page.getByRole("status")).toContainText(
      "Moved category Work to Channels.",
    );

    await expect
      .poll(async () => {
        const stored = await page.evaluate((key) => {
          return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
            channelsBlockIndex?: number;
          };
        }, SECTION_KEY);
        return stored.channelsBlockIndex;
      })
      .toBe(0);

    await page.reload();
    await page.getByTestId("channel-general").click();
    await expect
      .poll(async () => {
        const stored = await page.evaluate((key) => {
          return JSON.parse(window.localStorage.getItem(key) ?? "null") as {
            channelsBlockIndex?: number;
          };
        }, SECTION_KEY);
        return stored.channelsBlockIndex;
      })
      .toBe(0);
  });
});
