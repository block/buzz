import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Exercises the markdown-attachment viewer end-to-end through the mock Tauri
// bridge: upload a `.md` file → send → FileCard opens the in-app markdown
// viewer panel (not the download dialog) → Preview renders, Code shows the
// source, Download still works from the panel header.
//
// The attachment URL deliberately mirrors production shape: the relay stores
// extension-less text as `{sha256}.bin` (markdown has no magic bytes), so the
// `.md` identity lives only in the imeta filename. The mock upload descriptor
// reproduces that.

const RELAY_HTTP_URL =
  process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";
const DOC_SHA = "b".repeat(64);
const DOC_URL = `${RELAY_HTTP_URL}/media/${DOC_SHA}.bin`;
const DOC_MARKDOWN = [
  "# Release Notes",
  "",
  "Some **bold** text and a table:",
  "",
  "| Feature | Works |",
  "| --- | --- |",
  "| Headings | Yes |",
  "",
  "```js",
  'console.log("hi");',
  "```",
  "",
].join("\n");

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    // Route attach through the DOM file input (Playwright's filechooser)
    // instead of the native pick_and_upload_media dialog path.
    deferredComposerUploads: true,
    uploadDescriptors: [
      {
        url: DOC_URL,
        sha256: DOC_SHA,
        size: DOC_MARKDOWN.length,
        type: "application/octet-stream",
        uploaded: Math.floor(Date.now() / 1000),
        filename: "release-notes.md",
      },
    ],
  });
  // The bridge's `fetch_media_bytes` mock fetches the URL in-browser; serve
  // the document body from the spec instead of a real relay.
  await page.route(`**/media/${DOC_SHA}.bin`, (route) =>
    route.fulfill({
      body: DOC_MARKDOWN,
      contentType: "application/octet-stream",
    }),
  );
});

async function sendMarkdownAttachment(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await attachAndSendMarkdown(page);
}

/** Attach + send in the already-open channel (mock re-serves one descriptor,
 * so every send yields a card with the same document URL). */
async function attachAndSendMarkdown(page: Page) {
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "Attach file" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.from(DOC_MARKDOWN),
    mimeType: "text/markdown",
    name: "release-notes.md",
  });
  await expect(page.getByTestId("message-composer")).toContainText(
    "release-notes.md",
  );
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);
}

test("markdown attachment sent as a thread reply keeps the viewer action", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.locator('[data-testid^="reply-message-"]').first().click({
    force: true,
  });
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    threadPanel.getByRole("button", { name: "Attach file" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.from(DOC_MARKDOWN),
    mimeType: "text/markdown",
    name: "release-notes.md",
  });
  await threadPanel.getByTestId("send-message").click();
  await expect(threadPanel.getByText("Sending")).toHaveCount(0);

  const card = threadPanel.getByTestId("file-card").last();
  await expect(card).toContainText("release-notes.md");
  await expect(card).toHaveAttribute("aria-label", "Open release-notes.md");
});

test("markdown attachment opens the in-app viewer with Preview/Code toggle", async ({
  page,
}) => {
  await sendMarkdownAttachment(page);

  // The card advertises open-in-viewer, not download.
  const card = page.getByTestId("file-card").last();
  await expect(card).toContainText("release-notes.md");
  await expect(card).toHaveAttribute("aria-label", "Open release-notes.md");
  await card.click();

  // The viewer panel opens with the rendered document (no download dialog).
  const panel = page.getByTestId("markdown-doc-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("release-notes.md");
  await expect(
    panel.getByRole("heading", { name: "Release Notes" }),
  ).toBeVisible();
  // GFM table rendered as a real table, not pipes.
  await expect(panel.locator("table")).toContainText("Headings");
  const commands = () =>
    page.evaluate(
      () =>
        (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
          .__BUZZ_E2E_COMMANDS__ ?? [],
    );
  expect(await commands()).not.toContain("download_file");

  // Code view shows the raw source.
  await page.getByTestId("markdown-doc-view-code").click();
  await expect(page.getByTestId("markdown-doc-code")).toContainText(
    "# Release Notes",
  );
  await page.getByTestId("markdown-doc-view-preview").click();
  await expect(
    panel.getByRole("heading", { name: "Release Notes" }),
  ).toBeVisible();

  // Download stays available from the panel header.
  await page.getByTestId("markdown-doc-download").click();
  await expect.poll(commands).toContain("download_file");

  // Close returns to the plain channel view.
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("markdown-doc-panel")).toHaveCount(0);
});

test("narrow layout moves focus into the panel and returns it to the card", async ({
  page,
}) => {
  await sendMarkdownAttachment(page);

  // Below the split-pane threshold the panel replaces the channel section,
  // unmounting the focused attachment card — the focus contract under test.
  await page.setViewportSize({ width: 560, height: 720 });

  const card = page.getByTestId("file-card").last();
  await card.focus();
  await expect(card).toBeFocused();
  await page.keyboard.press("Enter");

  // Focus lands on the panel's close control, not <body>.
  const panel = page.getByTestId("markdown-doc-panel");
  await expect(panel).toBeVisible();
  await expect(page.getByTestId("auxiliary-panel-close")).toBeFocused();

  // Escape closes the panel and hands focus back to the invoking card —
  // overriding the remounted channel's composer autofocus.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("markdown-doc-panel")).toHaveCount(0);
  await expect(page.getByTestId("file-card").last()).toBeFocused();
});

test("close restores focus to the invoking card when the document appears twice", async ({
  page,
}) => {
  // The same attachment in two messages gives two cards with one URL, so a
  // URL-only anchor would always restore the first DOM match. The invoking
  // card's recorded identity must win.
  await sendMarkdownAttachment(page);
  await attachAndSendMarkdown(page);

  await page.setViewportSize({ width: 560, height: 720 });

  const docCards = page.locator(
    `[data-testid="file-card"][data-doc-url="${DOC_URL}"]`,
  );
  await expect(docCards).toHaveCount(2);

  // Open from the SECOND card.
  await docCards.nth(1).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("auxiliary-panel-close")).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("markdown-doc-panel")).toHaveCount(0);
  await expect(docCards).toHaveCount(2);
  await expect(docCards.nth(1)).toBeFocused();
  await expect(docCards.nth(0)).not.toBeFocused();
});

test("open document survives reload and back/forward navigation", async ({
  page,
}) => {
  await sendMarkdownAttachment(page);
  await page.getByTestId("file-card").last().click();

  const panel = () => page.getByTestId("markdown-doc-panel");
  await expect(
    panel().getByRole("heading", { name: "Release Notes" }),
  ).toBeVisible();

  // The document lives in the URL (`doc`/`docName` params), so a reload
  // must restore the open panel with its content.
  await page.reload();
  await expect(
    panel().getByRole("heading", { name: "Release Notes" }),
  ).toBeVisible();

  // Opening the panel pushed a history entry: back closes it, forward
  // restores it — the advertised back/forward contract.
  await page.goBack();
  await expect(page.getByTestId("markdown-doc-panel")).toHaveCount(0);
  // Rehydrating the channel must not downgrade a known markdown attachment to
  // Download while relay-origin discovery catches up.
  await expect(page.getByTestId("file-card").last()).toHaveAttribute(
    "aria-label",
    "Open release-notes.md",
  );
  await page.goForward();
  await expect(
    panel().getByRole("heading", { name: "Release Notes" }),
  ).toBeVisible();
});

test("a document over the native 2 MiB cap falls back to download", async ({
  page,
}) => {
  // The imeta `size` is untrusted and here it lies small — the card offers
  // the viewer. The served body is over the cap, so the (mocked) native
  // fetch refuses it mid-transfer and the panel must show the too-large
  // fallback instead of rendering, proving enforcement does not rest on
  // the advertised size.
  await page.unroute(`**/media/${DOC_SHA}.bin`);
  await page.route(`**/media/${DOC_SHA}.bin`, (route) =>
    route.fulfill({
      body: Buffer.alloc(2 * 1024 * 1024 + 1, 0x61),
      contentType: "application/octet-stream",
    }),
  );
  await sendMarkdownAttachment(page);

  const card = page.getByTestId("file-card").last();
  await expect(card).toHaveAttribute("aria-label", "Open release-notes.md");
  await card.click();

  const panel = page.getByTestId("markdown-doc-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("This file is too large to preview.");
  await expect(
    panel.getByRole("button", { name: "Download file" }),
  ).toBeVisible();
});

test("non-markdown attachments keep the download-card behavior", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  // Re-point the mock upload at a PDF: same flow, no viewer affordance.
  await page.evaluate(() => {
    const e2e = (
      window as Window & {
        __BUZZ_E2E__?: {
          mock?: { uploadDescriptors?: Array<Record<string, unknown>> };
        };
      }
    ).__BUZZ_E2E__;
    if (e2e?.mock) {
      e2e.mock.uploadDescriptors = [
        {
          url: `http://localhost:3000/media/${"c".repeat(64)}.pdf`,
          sha256: "c".repeat(64),
          size: 128,
          type: "application/pdf",
          uploaded: 1_700_000_000,
          filename: "report.pdf",
        },
      ];
    }
  });

  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "Attach file" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.from("pdf bytes"),
    mimeType: "application/pdf",
    name: "report.pdf",
  });
  await expect(page.getByTestId("message-composer")).toContainText(
    "report.pdf",
  );
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const card = page.getByTestId("file-card").last();
  await expect(card).toContainText("report.pdf");
  await expect(card).toHaveAttribute("aria-label", "Download report.pdf");
  await card.click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("download_file");
  await expect(page.getByTestId("markdown-doc-panel")).toHaveCount(0);
});
