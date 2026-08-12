import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Exercises the attachment file-viewer contract through the mock Tauri
// bridge: clicking a viewable FileCard (markdown/code/text) opens the
// right-side viewer panel with one tab per file, while non-viewable types
// keep the legacy download-on-click behavior. Content bytes travel through
// the mocked `fetch_media_bytes` command, which fetches the URL in-browser —
// each test routes its media URLs to canned bodies.

const MARKDOWN_SHA = "a".repeat(64);
const MARKDOWN_URL = `http://localhost:3000/media/${MARKDOWN_SHA}.md`;
const MARKDOWN_BODY = "# Release notes\n\nShipped the **file viewer** panel.";

const SCRIPT_SHA = "b".repeat(64);
const SCRIPT_URL = `http://localhost:3000/media/${SCRIPT_SHA}.sh`;
const SCRIPT_BODY = "#!/usr/bin/env bash\necho 'hello viewer'\n";

const PDF_SHA = "c".repeat(64);
const PDF_URL = `http://localhost:3000/media/${PDF_SHA}.pdf`;

function imetaTag(url: string, mime: string, sha: string, filename: string) {
  return [
    "imeta",
    `url ${url}`,
    `m ${mime}`,
    `x ${sha}`,
    `filename ${filename}`,
  ];
}

async function emitAttachmentMessage(
  page: Page,
  args: { content: string; extraTags: string[][] },
) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      (window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
      }) ??
        false),
  );
  return page.evaluate(({ content, extraTags }) => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable.");
    return emit({ channelName: "general", content, extraTags }).id;
  }, args);
}

async function invokedCommandCount(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (
        (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
          .__BUZZ_E2E_COMMANDS__ ?? []
      ).filter((invoked) => invoked === name).length,
    command,
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.route(MARKDOWN_URL, (route) =>
    route.fulfill({ body: MARKDOWN_BODY, contentType: "text/markdown" }),
  );
  await page.route(SCRIPT_URL, (route) =>
    route.fulfill({ body: SCRIPT_BODY, contentType: "text/x-shellscript" }),
  );
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("clicking a markdown FileCard opens the viewer, not a download", async ({
  page,
}) => {
  await emitAttachmentMessage(page, {
    content: `Here are the notes\n\n[release-notes.md](${MARKDOWN_URL})`,
    extraTags: [
      imetaTag(MARKDOWN_URL, "text/markdown", MARKDOWN_SHA, "release-notes.md"),
    ],
  });

  await page.getByTestId("file-card").last().click();

  const panel = page.getByTestId("file-viewer-panel");
  await expect(panel).toBeVisible();
  // Rendered markdown — the heading exists as a heading, not literal `#`.
  await expect(
    panel.getByRole("heading", { name: "Release notes" }),
  ).toBeVisible();
  await expect(page.getByTestId("file-viewer-tab")).toHaveText(
    "release-notes.md",
  );
  expect(await invokedCommandCount(page, "download_file")).toBe(0);
  // The tab strip is the panel's only title surface; a separate filename
  // heading would just repeat it. A rule must sit between the tabs and the
  // file, spanning the panel — assert geometry, not mere DOM presence.
  await expect(panel.getByRole("heading", { level: 2 })).toHaveCount(0);
  const divider = page.getByTestId("file-viewer-header-divider");
  await expect(divider).toBeVisible();
  const dividerBox = await divider.boundingBox();
  const tabBox = await page
    .getByTestId("file-viewer-tab")
    .first()
    .boundingBox();
  const bodyBox = await page.getByTestId("file-viewer-markdown").boundingBox();
  const panelBox = await panel.boundingBox();
  if (!dividerBox || !tabBox || !bodyBox || !panelBox) {
    throw new Error("file viewer layout boxes are unavailable");
  }
  expect(dividerBox.y).toBeGreaterThan(tabBox.y + tabBox.height);
  expect(dividerBox.y).toBeLessThanOrEqual(bodyBox.y);
  expect(dividerBox.width).toBeGreaterThan(panelBox.width - 2);

  // Copy puts the decoded file text on the clipboard.
  await page.getByTestId("file-viewer-copy").click();
  await expect
    .poll(() => invokedCommandCount(page, "copy_text_to_clipboard"))
    .toBe(1);

  // The explicit download affordance still downloads.
  await page.getByTestId("file-viewer-download").click();
  await expect.poll(() => invokedCommandCount(page, "download_file")).toBe(1);
});

test("opening a second file adds a tab; closing tabs restores and empties", async ({
  page,
}) => {
  await emitAttachmentMessage(page, {
    content: `Bundle\n\n[release-notes.md](${MARKDOWN_URL})\n\n[apply.sh](${SCRIPT_URL})`,
    extraTags: [
      imetaTag(MARKDOWN_URL, "text/markdown", MARKDOWN_SHA, "release-notes.md"),
      imetaTag(SCRIPT_URL, "application/x-sh", SCRIPT_SHA, "apply.sh"),
    ],
  });

  const cards = page.getByTestId("file-card");
  await cards.first().click();
  await expect(page.getByTestId("file-viewer-tab")).toHaveCount(1);
  await cards.nth(1).click();

  const tabs = page.getByTestId("file-viewer-tab");
  await expect(tabs).toHaveCount(2);
  // Second file is active: shell script renders through the code path.
  await expect(page.getByTestId("file-viewer-code")).toContainText(
    "echo 'hello viewer'",
  );

  // The strip scrolls inside a 36px header row, so its scrollbar must never
  // claim more than a hairline of that row. A horizontal scrollbar that
  // reserves space shows up as offsetHeight - clientHeight; overlay
  // scrollbars report 0, and both satisfy the cap.
  const scrollbarHeight = await page
    .getByTestId("file-viewer-tab-strip")
    .evaluate((el) => el.offsetHeight - el.clientHeight);
  expect(scrollbarHeight).toBeLessThanOrEqual(4);

  // Switch back to the markdown tab.
  await tabs.first().click();
  await expect(page.getByTestId("file-viewer-markdown")).toBeVisible();

  // Closing the active tab activates the neighbor.
  await page.getByTestId("file-viewer-tab-close").first().click();
  await expect(tabs).toHaveCount(1);
  await expect(page.getByTestId("file-viewer-code")).toBeVisible();

  // Closing the last tab closes the panel.
  await page.getByTestId("file-viewer-tab-close").click();
  await expect(page.getByTestId("file-viewer-panel")).toHaveCount(0);
});

test("non-viewable attachments keep download-on-click", async ({ page }) => {
  await emitAttachmentMessage(page, {
    content: `Budget\n\n[q3-budget.pdf](${PDF_URL})`,
    extraTags: [imetaTag(PDF_URL, "application/pdf", PDF_SHA, "q3-budget.pdf")],
  });

  await page.getByTestId("file-card").last().click();

  await expect.poll(() => invokedCommandCount(page, "download_file")).toBe(1);
  await expect(page.getByTestId("file-viewer-panel")).toHaveCount(0);
});

test("opening a thread supersedes the viewer panel", async ({ page }) => {
  const messageId = await emitAttachmentMessage(page, {
    content: `Notes\n\n[release-notes.md](${MARKDOWN_URL})`,
    extraTags: [
      imetaTag(MARKDOWN_URL, "text/markdown", MARKDOWN_SHA, "release-notes.md"),
    ],
  });

  await page.getByTestId("file-card").last().click();
  await expect(page.getByTestId("file-viewer-panel")).toBeVisible();

  // Reply in thread → the thread panel takes the aux slot.
  const row = page.locator(`[data-message-id="${messageId}"]`);
  await row.hover();
  await page.getByTestId(`reply-message-${messageId}`).click();

  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page.getByTestId("file-viewer-panel")).toHaveCount(0);

  // Opening a file from inside the thread borrows the slot, and closing the
  // viewer hands it back — the thread route state is never discarded.
  await page
    .getByTestId("message-thread-panel")
    .getByTestId("file-card")
    .first()
    .click();
  await expect(page.getByTestId("file-viewer-panel")).toBeVisible();
  await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);

  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page.getByTestId("file-viewer-panel")).toHaveCount(0);
});
