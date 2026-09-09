import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const TASK_CANVAS = `---
buzz_schema: channel-backed-task/v1
task:
  title: "Add status sounds to Berd Voice"
  description: "Move status-sound ownership into the Berd Voice runtime."
parent_channel: "buzz://channel/parent"
originating_thread: "buzz://message?channel=parent&id=origin"
branch:
  repository: "https://github.com/block/berd"
  name: "jtennant/berd-voice-status-sounds"
---`;

test("loading a corrected canvas refreshes the sidebar task metadata", async ({
  page,
}) => {
  await installMockBridge(page, { canvasContent: TASK_CANVAS });
  await page.goto("/");
  const row = page.getByTestId("channel-engineering");
  await expect(row.getByTestId("task-channel-status")).toBeVisible();
  await page.evaluate(() => {
    const w = window as typeof window & {
      __BUZZ_E2E_QUERY_CLIENT__: import("@tanstack/react-query").QueryClient;
    };
    const client = w.__BUZZ_E2E_QUERY_CLIENT__;
    client.setQueriesData(
      { queryKey: ["channel-backed-task-canvases"] },
      () => ({}),
    );
  });
  await expect(row.getByTestId("task-channel-status")).toHaveCount(0);
  await row.click();
  await expect(row.getByTestId("task-channel-status")).toBeVisible();
});

test("a compact task header sits above the conversation with origin at its start", async ({
  page,
}) => {
  await installMockBridge(page, { canvasContent: TASK_CANVAS });
  await page.goto("/");
  await page.getByTestId("channel-engineering").click();

  await expect(page.getByTestId("task-overview")).toContainText(
    "Add status sounds to Berd Voice",
  );
  await expect(page.getByTestId("task-view-changes")).toHaveCount(0);
  await expect(page.getByTestId("task-view-review")).toHaveCount(0);
  await expect(page.getByTestId("task-overview")).toContainText(
    "jtennant/berd-voice-status-sounds",
  );
  await expect(page.getByTestId("task-work-tree")).toHaveCount(0);
  await expect(page.getByTestId("task-view-tabs")).toHaveCount(0);
  await expect(page.getByTestId("message-timeline")).toBeVisible();
  await expect(
    page.getByTestId("task-overview").locator("summary, details"),
  ).toHaveCount(0);
  await expect(page.getByTestId("task-overview")).not.toContainText("Parent");
  await expect(page.getByTestId("task-overview")).not.toContainText("Origin");
  await expect(page.getByTestId("task-origin-thread")).toContainText(
    "Continued from",
  );
  await expect(page.getByTestId("message-channel-intro")).toHaveCount(0);
  await expect(page.getByTestId("task-work-navigator")).toHaveCount(0);
});

test("task canvases nest channels beneath their parents", async ({ page }) => {
  const parentId = "cf63feec-21bb-5bf0-a2f8-0e4c3de8ec73";
  const childId = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";
  const grandchildId = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
  const canvas = (parent: string) =>
    TASK_CANVAS.replace("buzz://channel/parent", `buzz://channel/${parent}`);

  await installMockBridge(page, {
    canvasContentByChannelId: {
      [childId]: canvas(parentId),
      [grandchildId]: canvas(childId),
    },
  });
  await page.goto("/");

  const list = page.getByTestId("stream-list");
  await expect(list.getByTestId("channel-buzz")).toBeVisible();
  await expect(
    list.getByTestId("channel-buzz").getByTestId("task-channel-status"),
  ).toHaveCount(0);
  await expect(
    list.getByTestId("channel-engineering").getByTestId("task-channel-status"),
  ).toBeVisible();
  await expect(
    list.locator(
      '[data-channel-depth="1"] [data-testid="channel-engineering"]',
    ),
  ).toBeVisible();
  await expect(
    list.locator('[data-channel-depth="2"] [data-testid="channel-agents"]'),
  ).toBeVisible();
  await list.getByTestId("channel-buzz").click();
  await expect(page.getByTestId("task-work-tree")).toHaveCount(0);
  await expect(page.getByTestId("task-view-changes")).toHaveCount(0);
  await expect(page.getByTestId("task-view-review")).toHaveCount(0);
});

test("sidebar glyphs retain status colors in selected and inactive channels", async ({
  page,
}) => {
  await installMockBridge(page, { canvasContent: TASK_CANVAS });
  await page.goto("/");
  const row = page.getByTestId("channel-engineering");
  await expect(row.getByTestId("task-channel-status")).toBeVisible();
  await page.evaluate(() => {
    const w = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload: unknown,
          options: unknown,
        ) => Promise<unknown>;
      };
      __TASK_PR__: unknown[];
    };
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = (command, payload, options) =>
      command === "get_task_github"
        ? Promise.resolve(w.__TASK_PR__)
        : original(command, payload, options);
  });
  for (const [state, isDraft, label, color] of [
    [null, false, "Branch", "muted-foreground"],
    ["OPEN", true, "Draft PR", "muted-foreground"],
    ["OPEN", false, "Open PR", "green-600"],
    ["MERGED", false, "Merged PR", "purple-600"],
    ["CLOSED", false, "Closed PR", "red-600"],
  ] as const) {
    await page.evaluate(
      async ({ state, isDraft }) => {
        const w = window as typeof window & {
          __TASK_PR__: unknown[];
          __BUZZ_E2E_QUERY_CLIENT__: {
            invalidateQueries: (filter: {
              queryKey: string[];
            }) => Promise<void>;
          };
        };
        w.__TASK_PR__ = state
          ? [
              {
                state,
                isDraft,
                number: 308,
                title: "Status sounds",
                url: "https://github.com/block/berd/pull/308",
              },
            ]
          : [];
        await w.__BUZZ_E2E_QUERY_CLIENT__.invalidateQueries({
          queryKey: ["task-github"],
        });
      },
      { state, isDraft },
    );
    const glyph = row.getByRole("img", { name: new RegExp(`^${label}:`) });
    await expect(glyph).toBeVisible();
    await expect(glyph).toHaveClass(new RegExp(`text-${color}`));
    const iconClass = !state
      ? null
      : state === "MERGED"
        ? "lucide-git-merge"
        : state === "CLOSED"
          ? "lucide-git-pull-request-closed"
          : isDraft
            ? "lucide-git-pull-request-draft"
            : "lucide-git-pull-request";
    if (iconClass)
      await expect(glyph).toHaveClass(new RegExp(`(?:^| )${iconClass}(?: |$)`));
    await page.getByTestId("channel-engineering").click();
    const headerGlyph = page
      .getByTestId("task-overview")
      .locator(`svg[aria-label="${label}"]`);
    await expect(headerGlyph).toBeVisible();
    if (iconClass)
      await expect(headerGlyph).toHaveClass(
        new RegExp(`(?:^| )${iconClass}(?: |$)`),
      );
    else
      await expect(headerGlyph.locator('[stroke-dasharray="1 4"]')).toHaveCount(
        1,
      );
    // Resolve the CSS utility in the browser, not just its class attribute.
    for (const dark of [false, true]) {
      await page.evaluate(
        (dark) => document.documentElement.classList.toggle("dark", dark),
        dark,
      );
      const expectedColor = await glyph.evaluate(
        (_element, color) => {
          const sample = document.createElement("span");
          sample.style.color =
            color === "muted-foreground"
              ? "hsl(var(--muted-foreground))"
              : `var(--color-${color})`;
          document.body.append(sample);
          const result = getComputedStyle(sample).color;
          sample.remove();
          return result;
        },
        dark ? color.replace("600", "400") : color,
      );
      for (const channel of ["engineering", "agents"]) {
        await page.getByTestId(`channel-${channel}`).click();
        await expect(glyph).toHaveCSS("color", expectedColor);
        await expect(glyph.locator("..")).toHaveCSS("opacity", "1");
      }
    }
    if (!state)
      await expect(glyph.locator('[stroke-dasharray="1 4"]')).toHaveCount(1);
  }
});
