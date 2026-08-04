import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { expectCornerRadiusPx, expectSmoothCorners } from "../helpers/css";

// Exercises the generic file-attachment UI contract end-to-end through the
// mock Tauri bridge: paperclip upload → composer chip → send → FileCard in the
// timeline. This guards the frontend wiring (the riskiest, previously
// untested path). It does NOT prove the real relay store/serve round-trip —
// that lives in the Rust media + relay tests.

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    deferredComposerUploads: true,
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

async function chooseQuarterlyReport(page: Page) {
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "Attach image" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.from("quarterly report"),
    mimeType: "application/pdf",
    name: "quarterly-report.pdf",
  });
}

async function chooseLargeVideo(page: Page) {
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "Attach image" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.alloc(16 * 1024 * 1024, 1),
    mimeType: "video/mp4",
    name: "large-video.mp4",
  });
}

test("upload a file and see a FileCard in the timeline", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // The paperclip queues the local file without starting its upload.
  await chooseQuarterlyReport(page);

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
  await expectCornerRadiusPx(card, 16);
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

test("sends immediately and keeps upload progress across channels", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    const e2e = (
      window as Window & {
        __BUZZ_E2E__?: { mock?: { uploadDelayMs?: number } };
      }
    ).__BUZZ_E2E__;
    if (e2e?.mock) e2e.mock.uploadDelayMs = 1_000;
  });
  await page.getByTestId("channel-general").click();
  await chooseQuarterlyReport(page);

  await expect(page.getByTestId("composer-upload-progress")).toHaveCount(0);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-composer")).not.toContainText(
    "quarterly-report.pdf",
  );
  await expect(page.getByTestId("composer-upload-progress")).toBeVisible();

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("composer-upload-progress")).toBeVisible();
  await expect(page.getByTestId("composer-upload-progress")).toHaveCount(0, {
    timeout: 5_000,
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("file-card").last()).toContainText(
    "quarterly-report.pdf",
  );
});

test("shows upload feedback before transferring a large file", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    const e2e = (
      window as Window & {
        __BUZZ_E2E__?: { mock?: { uploadDelayMs?: number } };
      }
    ).__BUZZ_E2E__;
    if (e2e?.mock) e2e.mock.uploadDelayMs = 5_000;
  });
  await page.getByTestId("channel-general").click();
  await chooseLargeVideo(page);

  const progress = page.getByTestId("composer-upload-progress");
  await Promise.all([
    page.getByTestId("send-message").click(),
    expect(progress).toBeVisible({ timeout: 800 }),
  ]);
  await expect(progress).toHaveAttribute("aria-label", "Preparing");
  await expect(page.getByTestId("composer-upload-spinner")).toBeVisible();
  await expect(page.getByTestId("composer-upload-percentage")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
                command: string;
                payload: { rawByteLength?: number } | null;
              }>;
            }
          ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
      ),
    )
    .toContainEqual({
      command: "upload_media_bytes_raw",
      payload: { rawByteLength: 16 * 1024 * 1024 },
    });

  const uploadId = "background-media-upload-0-0";
  await page.evaluate(async (id) => {
    await window.__BUZZ_E2E_EMIT_MEDIA_UPLOAD_PHASE__?.({
      id,
      phase: "processing-video",
    });
  }, uploadId);
  await expect(progress).toHaveAttribute("aria-label", "Processing");
  await waitForAnimations(page);
  const processingPhaseBox = await page
    .getByTestId("composer-upload-phase")
    .boundingBox();
  const processingStatusBox = await page
    .getByTestId("composer-upload-status")
    .boundingBox();
  expect(processingPhaseBox).not.toBeNull();
  expect(processingStatusBox).not.toBeNull();
  expect(
    (processingStatusBox?.x ?? 0) -
      ((processingPhaseBox?.x ?? 0) + (processingPhaseBox?.width ?? 0)),
  ).toBeGreaterThanOrEqual(3);
  await expect(page.getByTestId("composer-upload-spinner")).toBeVisible();
  await expect(page.getByTestId("composer-upload-percentage")).toHaveCount(0);

  await page.evaluate(async (id) => {
    await window.__BUZZ_E2E_EMIT_MEDIA_UPLOAD_PHASE__?.({
      id,
      phase: "uploading",
    });
    await window.__BUZZ_E2E_EMIT_MEDIA_UPLOAD_PROGRESS__?.({
      id,
      sent: 42,
      total: 100,
    });
  }, uploadId);
  await expect(progress).toHaveAttribute("aria-label", "Uploading 42%");
  await waitForAnimations(page);
  await expect(page.getByTestId("composer-upload-spinner")).toHaveCount(0);
  await expect(page.getByTestId("composer-upload-percentage")).toHaveText(
    "42%",
  );

  await page.getByTestId("composer-upload-cancel").click();
});

test("canceling a background upload prevents the message from publishing", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    const e2e = (
      window as Window & {
        __BUZZ_E2E__?: { mock?: { uploadDelayMs?: number } };
      }
    ).__BUZZ_E2E__;
    if (e2e?.mock) e2e.mock.uploadDelayMs = 1_000;
  });
  await page.getByTestId("channel-general").click();
  await chooseQuarterlyReport(page);
  await page.getByTestId("send-message").click();

  await page.getByTestId("composer-upload-cancel").click();
  await expect(page.getByTestId("composer-upload-progress")).toHaveCount(0);
  await page.waitForTimeout(1_100);
  await expect(page.getByTestId("file-card")).toHaveCount(0);
});

test("upload progress floats above the dock and lifts Jump to latest", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    const e2e = (
      window as Window & {
        __BUZZ_E2E__?: { mock?: { uploadDelayMs?: number } };
      }
    ).__BUZZ_E2E__;
    if (e2e?.mock) e2e.mock.uploadDelayMs = 2_000;
  });
  await page.getByTestId("channel-deep-history").click();

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await timeline.evaluate((element) => {
    element.scrollTop = Math.max(500, element.scrollHeight / 2);
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  const jumpToLatest = page.getByTestId("message-scroll-to-latest");
  await expect(jumpToLatest).toBeVisible();
  const restingBox = await jumpToLatest.boundingBox();

  await chooseQuarterlyReport(page);
  await page.getByTestId("send-message").click();
  const uploadMotion = page.getByTestId("composer-upload-progress-motion");
  await expect(uploadMotion).toBeVisible();
  await timeline.evaluate((element) => {
    element.scrollTop = Math.max(500, element.scrollHeight / 2);
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(jumpToLatest).toBeVisible();
  await page.waitForTimeout(250);

  const [uploadBox, dockBackdropBox, liftedBox] = await Promise.all([
    uploadMotion.boundingBox(),
    page.getByTestId("composer-dock-backdrop").boundingBox(),
    jumpToLatest.boundingBox(),
  ]);
  expect(restingBox).not.toBeNull();
  expect(uploadBox).not.toBeNull();
  expect(dockBackdropBox).not.toBeNull();
  expect(liftedBox).not.toBeNull();
  expect((dockBackdropBox?.y ?? 0) + 1).toBeGreaterThanOrEqual(
    (uploadBox?.y ?? 0) + (uploadBox?.height ?? 0),
  );
  expect((liftedBox?.y ?? 0) + (liftedBox?.height ?? 0)).toBeLessThanOrEqual(
    uploadBox?.y ?? 0,
  );
  expect(liftedBox?.y ?? 0).toBeLessThan((restingBox?.y ?? 0) - 10);

  await page.getByTestId("composer-upload-cancel").click();
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
  const overlay = dropZone.getByTestId("drop-zone-overlay");
  const label = dropZone.getByTestId("drop-zone-label");
  await expect(overlay).toBeVisible();
  await expect(label).toContainText("Drop files to upload");

  const [dropZoneBox, overlayBox, overlayStyles, stacking] = await Promise.all([
    dropZone.boundingBox(),
    overlay.boundingBox(),
    page.evaluate(() => {
      const overlayElement = document.querySelector<HTMLElement>(
        '[data-testid="drop-zone-overlay"]',
      );
      const contentSurface = document.querySelector<HTMLElement>(
        "[data-buzz-content-surface]",
      );
      if (!(overlayElement && contentSurface)) return null;
      const overlayStyle = getComputedStyle(overlayElement);
      return {
        backdropFilter: overlayStyle.backdropFilter,
        containerRadius: getComputedStyle(contentSurface).borderRadius,
        overlayRadius: overlayStyle.borderRadius,
      };
    }),
    page.evaluate(() => {
      const overlayElement = document.querySelector<HTMLElement>(
        '[data-testid="drop-zone-overlay"]',
      );
      const composerOverlayElement = document.querySelector<HTMLElement>(
        '[data-testid="channel-composer-overlay"]',
      );
      if (!(overlayElement && composerOverlayElement)) return null;
      return {
        composer: Number.parseInt(
          getComputedStyle(composerOverlayElement).zIndex,
          10,
        ),
        dropZone: Number.parseInt(getComputedStyle(overlayElement).zIndex, 10),
      };
    }),
  ]);

  expect(overlayBox).toEqual(dropZoneBox);
  expect(overlayStyles).not.toBeNull();
  expect(overlayStyles?.overlayRadius).toBe(overlayStyles?.containerRadius);
  expect(overlayStyles?.backdropFilter).toContain("blur");
  expect(stacking).not.toBeNull();
  expect(stacking?.dropZone).toBeGreaterThan(stacking?.composer ?? 0);

  await dropZone.dispatchEvent("drop", { dataTransfer });
  await expect(page.getByTestId("message-composer")).toContainText(
    "quarterly-report.pdf",
  );
});

for (const theme of ["buzz", "buzz-dark", "github-light", "github-dark"]) {
  test(`drop prompt has accessible text contrast in ${theme}`, async ({
    page,
  }) => {
    await page.goto("/");
    await page.evaluate((selectedTheme) => {
      window.localStorage.setItem("buzz-theme", selectedTheme);
    }, theme);
    await page.reload();
    await page.getByTestId("channel-general").click();

    const dataTransfer = await page.evaluateHandle(() => {
      const transfer = new DataTransfer();
      transfer.items.add(
        new File(["contrast check"], "contrast-check.txt", {
          type: "text/plain",
        }),
      );
      return transfer;
    });
    const dropZone = page.getByTestId("channel-drop-zone");
    await dropZone.dispatchEvent("dragenter", { dataTransfer });

    const contrastRatio = await dropZone
      .getByTestId("drop-zone-label")
      .evaluate((element) => {
        const parseRgb = (value: string) =>
          (value.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
        const luminance = (color: number[]) =>
          color
            .map((channel) => {
              const value = channel / 255;
              return value <= 0.04045
                ? value / 12.92
                : ((value + 0.055) / 1.055) ** 2.4;
            })
            .reduce(
              (sum, channel, index) =>
                sum + channel * [0.2126, 0.7152, 0.0722][index],
              0,
            );
        const style = getComputedStyle(element);
        const foreground = luminance(parseRgb(style.color));
        const background = luminance(parseRgb(style.backgroundColor));
        return (
          (Math.max(foreground, background) + 0.05) /
          (Math.min(foreground, background) + 0.05)
        );
      });

    expect(contrastRatio).toBeGreaterThanOrEqual(4.5);
  });
}

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
