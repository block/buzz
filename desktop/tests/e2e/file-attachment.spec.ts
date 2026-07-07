import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { expectCornerRadiusPx, expectSmoothCorners } from "../helpers/css";

// Exercises the generic file-attachment UI contract end-to-end through the
// mock Tauri bridge: paperclip upload → composer chip → send → FileCard in the
// timeline. This guards the frontend wiring (the riskiest, previously
// untested path). It does NOT prove the real relay store/serve round-trip —
// that lives in the Rust media + relay tests.

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    uploadDescriptors: [
      {
        url: `https://mock.relay/media/${"a".repeat(64)}.pdf`,
        sha256: "a".repeat(64),
        size: 12345,
        type: "application/pdf",
        uploaded: Math.floor(Date.now() / 1000),
        filename: "quarterly-report.pdf",
      },
    ],
  });
});

test("upload a file and see a FileCard in the timeline", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Paperclip → mocked pick_and_upload_media returns the PDF descriptor.
  await page.getByRole("button", { name: "Attach image" }).click();

  // The composer shows a chip with the original filename.
  await expect(page.getByTestId("message-composer")).toContainText(
    "quarterly-report.pdf",
  );

  // Send the (attachment-only) message.
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  // A FileCard renders in the timeline: a button carrying the filename. It
  // downloads via the native `download_file` command (HTTP inside the app's
  // tunnel + save dialog), NOT a plain `<a download>` link — a bare link
  // escapes the webview to the OS browser and hits a corporate CDN page.
  const card = page.getByTestId("file-card").last();
  await expect(card).toBeVisible();
  await expectCornerRadiusPx(card, 16, { scaleWithRootFont: true });
  await expectSmoothCorners(card);
  await expect(card).toContainText("quarterly-report.pdf");

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
});

test("dropping a file on the channel column attaches it to the composer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const dataTransfer = await page.evaluateHandle(() => {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File(["quarterly report"], "quarterly-report.pdf", {
        type: "application/pdf",
      }),
    );
    return transfer;
  });

  const dropZone = page.getByTestId("channel-drop-zone");
  await dropZone.dispatchEvent("dragenter", { dataTransfer });
  await expect(dropZone.getByText("Drop files to upload")).toBeVisible();

  await dropZone.dispatchEvent("drop", { dataTransfer });
  await expect(page.getByTestId("message-composer")).toContainText(
    "quarterly-report.pdf",
  );
});

test("forum posts emit a FileCard for generic attachments, not a broken image", async ({
  page,
}) => {
  // Regression guard for the ForumComposer bug: it used to hand-build content
  // as `![image](url)` for every non-video attachment (and omit the `filename`
  // imeta tag), so a PDF posted in a forum rendered as a broken inline image
  // and lost its label. The fix routes forum/notes posts through the same
  // `buildOutgoingMessage` builder as chat. This test would fail (no FileCard)
  // if ForumComposer ever drifts back to hand-building media markdown.
  await page.goto("/");

  // "watercooler" is a seeded forum the mock identity is a member of.
  await page.getByTestId("channel-watercooler").click();

  // Open the new-post composer ("Start a new post...").
  await page.getByRole("button", { name: "Start a new post..." }).click();

  // Paperclip → mocked pick_and_upload_media returns the PDF descriptor.
  await page.getByRole("button", { name: "Attach image" }).click();

  // Submit the (attachment-only) forum post.
  await page.getByTestId("send-message").click();

  // The post renders through the shared Markdown component as a FileCard —
  // a button carrying the filename that downloads via the native
  // `download_file` command — NOT an inline image and NOT a bare link.
  const card = page.getByTestId("file-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("quarterly-report.pdf");

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
});
