import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const token = "0123456789abcdef".repeat(8);
const url = `https://example.com/reports/${token}`;
const prose =
  "Review the rollout notes and confirm that each owner can read the complete status without scrolling sideways. Keep the next action beside its owner, even when this description spans several lines.";
const content = `Table readability fixture

| Owner | Status and next action with enough detail to span multiple lines in a narrow pane |
| --- | --- |
| Alice | ${prose} |
| Bob | [Read the complete rollout notes and review checklist](${url}) and then confirm the next step. |
| Token | ${token} |
| Link | <${url}> |
| Code | \`git diff --check\` and **review** the result. |

Surrounding paragraph stays in the message layout.`;

for (const surface of ["channel", "thread"] as const) {
  test(`markdown tables wrap and stay contained in the ${surface}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 1280, height: 1440 });
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.waitForFunction(() =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
      }),
    );
    const root = await page.evaluate((body) => {
      const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: body,
      });
      if (!root) throw new Error("Mock message was not emitted");
      return root.id;
    }, content);

    const timelineMessage = page
      .getByTestId("message-timeline")
      .locator(`[data-message-id="${root}"]`);
    await expect(timelineMessage).toBeVisible();
    if (surface === "thread") {
      await timelineMessage.hover();
      await page.getByTestId(`reply-message-${root}`).click();
      await expect(page.getByTestId("message-thread-panel")).toBeVisible();
    }
    const scope = page.getByTestId(
      surface === "thread" ? "message-thread-panel" : "message-timeline",
    );
    const markdown = scope
      .locator(".message-markdown")
      .filter({ hasText: "Table readability fixture" });
    const block = markdown.locator("[data-table-block]");
    await expect(block).toBeVisible();
    await page.mouse.move(0, 0);
    await waitForAnimations(page);
    await markdown.screenshot({ path: testInfo.outputPath(`${surface}.png`) });
    const metrics = await block.evaluate((element) => {
      const table = element.querySelector("table");
      if (!table) throw new Error("Semantic table missing");
      const label = document.createRange();
      label.selectNodeContents(table.rows[0].cells[0]);
      return {
        labelLines: label.getClientRects().length,
        width: element.clientWidth,
        scrollWidth: element.scrollWidth,
        tableWidth: table.getBoundingClientRect().width,
        alignments: Array.from(
          table.querySelectorAll("th, td"),
          (cell) => getComputedStyle(cell).verticalAlign,
        ),
        rowHeight: table.rows[1].getBoundingClientRect().height,
        lineHeight: Number.parseFloat(getComputedStyle(table).lineHeight),
        pageWidth: document.documentElement.clientWidth,
        pageScrollWidth: document.documentElement.scrollWidth,
      };
    });
    await testInfo.attach("layout", {
      body: JSON.stringify(metrics, null, 2),
      contentType: "application/json",
    });
    expect(metrics.width).toBeGreaterThan(250);
    if (surface === "thread") expect(metrics.width).toBeLessThan(500);
    expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.width + 1);
    expect(metrics.tableWidth).toBeLessThanOrEqual(metrics.width + 1);
    expect(metrics.alignments.every((value) => value === "top")).toBe(true);
    expect(metrics.labelLines).toBe(1);
    expect(metrics.rowHeight).toBeGreaterThan(metrics.lineHeight * 2);
    expect(metrics.pageScrollWidth).toBe(metrics.pageWidth);
    await expect(block.locator("tbody tr")).toHaveCount(5);
    await expect(block.getByRole("link")).toHaveCount(2);
    for (const link of await block.getByRole("link").all()) {
      await expect(link).toHaveAttribute("href", url);
    }
    await expect(block.locator("code")).toHaveText("git diff --check");
    await expect(
      block.locator("td").filter({ hasText: token }).first(),
    ).toHaveText(token);
    await expect(markdown.locator("p").last()).toHaveText(
      "Surrounding paragraph stays in the message layout.",
    );
  });
}

test("unavoidably wide tables scroll locally without losing cells", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "general",
    }),
  );
  const columns = Array.from({ length: 40 }, (_, i) => `C${i}`);
  const wide = [columns, columns.map(() => "---"), columns]
    .map((row) => `| ${row.join(" | ")} |`)
    .join("\n");
  await page.evaluate((body) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: `Wide table fixture\n\n${body}`,
    });
  }, wide);
  const block = page
    .getByTestId("message-timeline")
    .locator(".message-markdown")
    .filter({ hasText: "Wide table fixture" })
    .locator("[data-table-block]");
  await expect(block.locator("td")).toHaveCount(40);
  const metrics = await block.evaluate((element) => {
    element.scrollLeft = element.scrollWidth;
    return {
      width: element.clientWidth,
      scrollWidth: element.scrollWidth,
      scrollLeft: element.scrollLeft,
      overflow: getComputedStyle(element).overflowX,
      pageWidth: document.documentElement.clientWidth,
      pageScrollWidth: document.documentElement.scrollWidth,
    };
  });
  expect(metrics.scrollWidth).toBeGreaterThan(metrics.width);
  expect(metrics.scrollLeft).toBeGreaterThan(0);
  expect(metrics.overflow).toBe("auto");
  expect(metrics.pageScrollWidth).toBe(metrics.pageWidth);
  await expect(block.locator("td").last()).toHaveText("C39");
});
