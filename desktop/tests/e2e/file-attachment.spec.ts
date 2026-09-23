import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { expectCornerRadiusPx, expectSmoothCorners } from "../helpers/css";

async function openMoreActionsMenu(page: Page, messageId: string) {
  const row = page.locator(`[data-message-id="${messageId}"]`);
  await row.hover();
  await page.getByTestId(`more-actions-${messageId}`).click();
  await expect(page.locator('[role="menuitem"]').first()).toBeVisible({
    timeout: 5_000,
  });
}

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
    page.getByRole("button", { name: "Attach file" }).click(),
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
    page.getByRole("button", { name: "Attach file" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.alloc(16 * 1024 * 1024, 1),
    mimeType: "video/mp4",
    name: "large-video.mp4",
  });
}

async function choosePhoto(page: Page) {
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "Attach file" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.from("photo"),
    mimeType: "image/png",
    name: "photo.png",
  });
}

const PHOTO_FILE = {
  buffer: Buffer.from("photo"),
  mimeType: "image/png",
  name: "photo.png",
};

async function uploadCommandCount(page: Page) {
  return page.evaluate(
    () =>
      (
        (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
          .__BUZZ_E2E_COMMANDS__ ?? []
      ).filter((command) => command === "upload_media_bytes_raw").length,
  );
}

async function e2eCommands(page: Page) {
  return page.evaluate(
    () =>
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? [],
  );
}

async function openFilePreviewAndDownload(page: Page, filename: string) {
  const card = page.getByTestId("file-card").last();
  await expect(card).toBeVisible();
  await expect(card).toContainText(filename);

  await card.click();
  const preview = page.getByTestId("file-preview-dialog");
  await expect(preview).toBeVisible();
  await expect(preview).toContainText(filename);
  await expect(preview.getByTestId("file-preview-unavailable")).toBeVisible();

  // Click opens the preview — download is a separate action.
  expect(await e2eCommands(page)).not.toContain("download_file");

  await preview.getByTestId("file-preview-download").click();
  await expect.poll(() => e2eCommands(page)).toContain("download_file");

  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);
  return card;
}

test("picker survives cancel, same-file retry, and multiple selection", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const attach = page.getByRole("button", { name: "Attach file" });

  // Model cancel/no selection, then immediately reopen. The composer must
  // reuse its one mounted input rather than creating competing detached ones.
  const [canceledChooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    attach.click(),
  ]);
  await canceledChooser.setFiles([]);

  await choosePhoto(page);
  await expect.poll(() => uploadCommandCount(page)).toBe(1);

  // Reset-before-open is load-bearing: without it browsers suppress `change`
  // when the same path remains selected.
  await choosePhoto(page);
  await expect.poll(() => uploadCommandCount(page)).toBe(2);

  const [multipleChooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    attach.click(),
  ]);
  await multipleChooser.setFiles([
    PHOTO_FILE,
    { ...PHOTO_FILE, buffer: Buffer.from("second photo"), name: "other.png" },
  ]);
  await expect.poll(() => uploadCommandCount(page)).toBe(4);
});

test("photos upload before Send without a queued spoiler control", async ({
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
  await choosePhoto(page);

  await expect(page.getByTestId("upload-progress")).toBeVisible();
  await expect(page.getByTestId("composer-queued-video-spoiler")).toHaveCount(
    0,
  );
  // Photos upload immediately, so they are in neither `pendingImeta` nor the
  // queued list until the upload lands: Send stays blocked so the message
  // cannot publish without the attachment.
  await expect(page.getByTestId("send-message")).toBeDisabled();
  await expect(page.getByTestId("upload-progress")).toHaveCount(0, {
    timeout: 5_000,
  });
  await expect(page.getByTestId("composer-upload-progress")).toHaveCount(0);
  await expect(page.getByTestId("send-message")).toBeEnabled();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("upload_media_bytes_raw");
});

test("opening edit during an immediate photo upload preserves the draft", async ({
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
  await choosePhoto(page);
  await expect(page.getByTestId("upload-progress")).toBeVisible();

  await openMoreActionsMenu(page, "mock-general-welcome");
  await page.getByTestId("edit-message-mock-general-welcome").click();

  // Edit entry is rejected while the compacted draft cannot represent the
  // reserved upload slot. The upload remains current and lands in the draft.
  await expect(page.getByTestId("edit-target")).toHaveCount(0);
  await expect(page.getByTestId("upload-progress")).toBeVisible();
  await expect(page.getByTestId("upload-progress")).toHaveCount(0, {
    timeout: 5_000,
  });
  await expect(page.getByTestId("message-composer")).toContainText(
    "quarterly-report.pdf",
  );

  // Once settled, the same edit action enters edit mode normally.
  await openMoreActionsMenu(page, "mock-general-welcome");
  await page.getByTestId("edit-message-mock-general-welcome").click();
  await expect(page.getByTestId("edit-target")).toBeVisible();
});

test("upload a file and see a FileCard in the timeline", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Non-video files keep the established immediate-upload behavior.
  await chooseQuarterlyReport(page);

  // The composer shows a chip with the original filename.
  await expect(page.getByTestId("message-composer")).toContainText(
    "quarterly-report.pdf",
  );

  // Send the (attachment-only) message.
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  // A FileCard renders in the timeline: a button carrying the filename.
  // Click opens an in-app preview; Download in that window invokes the
  // native `download_file` command (HTTP inside the app's tunnel + save
  // dialog), NOT a plain `<a download>` link — a bare link escapes the
  // webview to the OS browser and hits a corporate CDN page.
  const card = page.getByTestId("file-card").last();
  await expect(card).toBeVisible();
  await expectCornerRadiusPx(card, 16);
  await expectSmoothCorners(card);
  await expect(card).toContainText("quarterly-report.pdf");

  await openFilePreviewAndDownload(page, "quarterly-report.pdf");
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
  await chooseLargeVideo(page);

  await expect(page.getByTestId("composer-upload-progress")).toHaveCount(0);
  await expect(page.getByTestId("composer-video-spoiler")).toHaveCount(0);
  const queuedSpoiler = page.getByTestId("composer-queued-video-spoiler");
  // Revealed on hover rather than removed from the DOM: the control stays
  // focusable so keyboard users can reach it, but is transparent and
  // click-through until the thumbnail is hovered or focused.
  await expect(queuedSpoiler).toHaveCSS("opacity", "0");
  await expect(queuedSpoiler).toHaveCSS("pointer-events", "none");
  await page.getByTestId("composer-queued-media-attachment").hover();
  await expect(queuedSpoiler).toBeVisible();
  await expect(queuedSpoiler).toHaveCSS("opacity", "1");
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-composer")).not.toContainText(
    "large-video.mp4",
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
  await chooseLargeVideo(page);
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

  await chooseLargeVideo(page);
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
  await page.getByRole("button", { name: "Attach file" }).click();

  // Submit the (attachment-only) forum post.
  await page.getByTestId("send-message").click();

  // The post renders through the shared Markdown component as a FileCard —
  // a button carrying the filename that opens an in-app preview. Download
  // from that window uses the native `download_file` command — NOT an
  // inline image and NOT a bare link.
  const card = page.getByTestId("file-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("quarterly-report.pdf");

  await openFilePreviewAndDownload(page, "quarterly-report.pdf");
});

test("a queued attachment can be removed without a mouse", async ({ page }) => {
  // Regression: the queued remove badge is revealed on hover, but hiding it
  // with `display: none` made it unfocusable, leaving keyboard-only users no
  // way to drop a queued video before sending.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await chooseLargeVideo(page);

  const queued = page.getByTestId("composer-queued-media-attachment");
  await expect(queued).toBeVisible();

  const remove = queued.getByRole("button", { name: "Remove attachment" });
  // Focusable while transparent — this is what `display: none` prevented.
  await remove.focus();
  await expect(remove).toBeFocused();
  // Focus reveals it, so the user can see what they are about to activate.
  await expect(remove).toHaveCSS("opacity", "1");

  await page.keyboard.press("Enter");
  await expect(queued).toHaveCount(0);
  await expect(page.getByTestId("message-composer")).not.toContainText(
    "large-video.mp4",
  );
});

test("an uploaded attachment's remove button is named and keyboard-operable", async ({
  page,
}) => {
  // Companion to the queued case: these badges are now in the tab order, so
  // every icon-only remove button needs an accessible name a screen reader can
  // read. Images and non-media files render through different branches, so
  // both are checked here.
  await installMockBridge(page, {
    deferredComposerUploads: true,
    uploadDescriptors: [
      {
        url: `https://mock.relay/media/${"b".repeat(64)}.png`,
        sha256: "b".repeat(64),
        size: 2048,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        dim: "320x200",
        filename: "photo.png",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const composer = page.getByTestId("message-composer");
  const remove = composer.getByRole("button", { name: "Remove attachment" });

  // Image attachment (MediaAttachmentItem).
  await choosePhoto(page);
  await expect(composer.getByTestId("composer-media-attachment")).toBeVisible();
  await expect(remove).toHaveCount(1);
  await remove.focus();
  await expect(remove).toBeFocused();
  await expect(remove).toHaveCSS("opacity", "1");
  await page.keyboard.press("Enter");
  await expect(composer.getByTestId("composer-media-attachment")).toHaveCount(
    0,
  );
});

test("a non-media attachment's remove button is named and keyboard-operable", async ({
  page,
}) => {
  // The file-card branch renders its own remove badge, so it needs the same
  // accessible name as the image and queued ones.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await chooseQuarterlyReport(page);

  const composer = page.getByTestId("message-composer");
  await expect(composer).toContainText("quarterly-report.pdf");

  const remove = composer.getByRole("button", { name: "Remove attachment" });
  await expect(remove).toHaveCount(1);
  await remove.focus();
  await expect(remove).toBeFocused();
  await expect(remove).toHaveCSS("opacity", "1");
  await page.keyboard.press("Enter");
  await expect(composer).not.toContainText("quarterly-report.pdf");
});

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(async () => {
      return page.evaluate((name) => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }) ??
          false
        );
      }, channelName);
    })
    .toBe(true);
}

function emitMockMessage(
  page: Page,
  channelName: string,
  content: string,
  extraTags: string[][],
) {
  return page.evaluate(
    ({ channelName, content, extraTags }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            extraTags?: string[][];
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) {
        throw new Error("Mock message emitter is unavailable.");
      }
      return emit({ channelName, content, extraTags });
    },
    { channelName, content, extraTags },
  );
}

test("markdown and HTML attachments preview in-app without navigating", async ({
  page,
}) => {
  const mdUrl = `https://mock.relay/media/${"1".repeat(64)}.md`;
  const htmlUrl = `https://mock.relay/media/${"2".repeat(64)}.html`;
  const mdBody = [
    "# Hello preview",
    "",
    "A **bold** note.",
    "",
    "```js",
    "const a = 1;",
    "const b = 2;",
    "```",
    "",
  ].join("\n");
  const htmlBody =
    '<!DOCTYPE html><html><head><style>@keyframes spin{to{transform:rotate(360deg)}}#spin{animation:spin 1s linear infinite;width:8px;height:8px;background:red}</style></head><body><script>window.__XSS__=1;document.documentElement.dataset.guestJs=\'1\';</script><h1>Live?</h1><button type="button" onclick="document.getElementById(\'panel\').hidden=false">Show tab</button><p id="panel" hidden>In-page nav</p><a href="#section">Go to section</a><h2 id="section">Section target</h2><a href="https://example.com/docs" target="_blank">External docs</a><a href="other.html">Other file</a><div id="spin"></div></body></html>';

  await page.route(mdUrl, (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/markdown",
      body: mdBody,
    }),
  );
  await page.route(htmlUrl, (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/html",
      body: htmlBody,
    }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", `[notes.md](${mdUrl})`, [
    [
      "imeta",
      `url ${mdUrl}`,
      "m text/markdown",
      `x ${"1".repeat(64)}`,
      "size 32",
      "filename notes.md",
    ],
  ]);

  const mdCard = page.getByTestId("file-card").filter({ hasText: "notes.md" });
  await expect(mdCard).toBeVisible();
  await mdCard.click();

  const preview = page.getByTestId("file-preview-dialog");
  await expect(preview).toBeVisible();
  const header = preview.getByTestId("file-preview-header");
  await expect(header).toHaveAttribute("data-header-layout", "toolbar");
  await expect(header.getByTestId("file-preview-tabs")).toBeVisible();
  await expect(
    preview.getByTestId("file-preview-body").getByTestId("file-preview-tabs"),
  ).toHaveCount(0);
  const markdownHeaderBox = await header.boundingBox();
  expect(markdownHeaderBox?.height ?? 999).toBeLessThan(80);
  await expect(
    preview
      .getByTestId("file-preview-footer")
      .getByTestId("file-preview-download"),
  ).toBeVisible();
  await expect(header.getByTestId("file-preview-download")).toHaveCount(0);

  const markdown = preview.getByTestId("file-preview-markdown");
  await expect(markdown).toBeVisible();
  await expect(markdown).toContainText("Hello preview");
  const fenceLines = markdown.locator("[data-line]");
  await expect(fenceLines).toHaveCount(2);
  await expect(fenceLines.nth(0)).toContainText("const a = 1;");
  await expect(fenceLines.nth(1)).toContainText("const b = 2;");
  await waitForAnimations(page);
  const firstLine = await fenceLines.nth(0).boundingBox();
  const secondLine = await fenceLines.nth(1).boundingBox();
  expect(firstLine && secondLine).toBeTruthy();
  expect(secondLine?.y ?? 0).toBeGreaterThan(firstLine?.y ?? 0);
  await expect(preview.locator("iframe")).toHaveCount(0);

  await preview.getByTestId("file-preview-tab-source").click();
  const mdSource = preview.getByTestId("file-preview-source");
  await expect(mdSource).toBeVisible();
  await expect(mdSource).toContainText("# Hello preview");
  await preview.getByTestId("file-preview-tab-preview").click();
  await expect(markdown).toBeVisible();
  await expect(markdown).toContainText("Hello preview");

  expect(await e2eCommands(page)).not.toContain("download_file");

  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);

  await emitMockMessage(page, "general", `[page.html](${htmlUrl})`, [
    [
      "imeta",
      `url ${htmlUrl}`,
      "m text/html",
      `x ${"2".repeat(64)}`,
      "size 96",
      "filename page.html",
    ],
  ]);

  const htmlCard = page
    .getByTestId("file-card")
    .filter({ hasText: "page.html" });
  await expect(htmlCard).toBeVisible();
  await htmlCard.click();

  await expect(preview).toBeVisible();
  await expect(header).toHaveAttribute("data-header-layout", "toolbar");
  await expect(header.getByTestId("file-preview-tabs")).toBeVisible();
  await expect(header.getByTestId("file-preview-fullscreen")).toBeVisible();
  const htmlHeaderBox = await header.boundingBox();
  expect(htmlHeaderBox?.height ?? 999).toBeLessThan(80);
  await expect(preview).toContainText("page.html");

  const iframe = preview.getByTestId("file-preview-html");
  await expect(iframe).toBeVisible();
  const sandbox = await iframe.getAttribute("sandbox");
  expect(sandbox ?? "").toContain("allow-scripts");
  expect(sandbox ?? "").toContain("allow-popups-to-escape-sandbox");
  expect(sandbox ?? "").not.toContain("allow-same-origin");
  expect(sandbox ?? "").not.toContain("allow-top-navigation");
  await expect(iframe).toHaveAttribute("src", /^blob:/);
  const iframeSrc = await iframe.getAttribute("src");
  expect(iframeSrc ?? "").not.toContain("/media/");
  expect(await iframe.getAttribute("srcdoc")).toBeFalsy();

  const frame = preview.frameLocator('[data-testid="file-preview-html"]');
  await expect(frame.getByRole("heading", { name: "Live?" })).toBeVisible();
  await expect(preview.getByRole("heading", { name: "Live?" })).toHaveCount(0);
  await expect(frame.locator("html")).toHaveAttribute("data-guest-js", "1");
  await frame.getByRole("button", { name: "Show tab" }).click();
  await expect(frame.getByText("In-page nav")).toBeVisible();
  await expect(frame.locator("#spin")).toBeVisible();

  await frame.getByRole("link", { name: "Go to section" }).click();
  await expect(
    frame.getByRole("heading", { name: "Section target" }),
  ).toBeInViewport();

  const parentUrl = page.url();
  const popupPromise = page
    .waitForEvent("popup", { timeout: 2000 })
    .catch(() => null);
  await frame.getByRole("link", { name: "External docs" }).click();
  const popup = await popupPromise;
  if (popup) await popup.close();
  expect(page.url()).toBe(parentUrl);

  await frame.getByRole("link", { name: "Other file" }).click();
  await expect(
    frame.getByText("links to other files cannot be opened here"),
  ).toBeVisible();

  const xssFlag = await page.evaluate(
    () => (window as Window & { __XSS__?: number }).__XSS__,
  );
  expect(xssFlag).toBeUndefined();

  // Unique origin: the parent must not be able to read frame globals.
  const parentReadOfFrame = await iframe.evaluate((node) => {
    const frameEl = node as HTMLIFrameElement;
    try {
      return frameEl.contentWindow
        ? (frameEl.contentWindow as Window & { __XSS__?: number }).__XSS__
        : undefined;
    } catch {
      return "blocked";
    }
  });
  expect(
    parentReadOfFrame === undefined || parentReadOfFrame === "blocked",
  ).toBe(true);

  await header.getByTestId("file-preview-fullscreen").click();
  await expect(preview).toHaveAttribute("data-fullscreen", "true");
  const fullscreenBox = await preview.boundingBox();
  const viewport = page.viewportSize();
  expect(fullscreenBox?.height ?? 0).toBeGreaterThan(
    (viewport?.height ?? 720) * 0.75,
  );
  const fullscreenSandbox = await iframe.getAttribute("sandbox");
  expect(fullscreenSandbox ?? "").toContain("allow-scripts");
  expect(fullscreenSandbox ?? "").not.toContain("allow-same-origin");
  const fullscreenSrc = await iframe.getAttribute("src");
  expect(fullscreenSrc ?? "").toMatch(/^blob:/);
  expect(fullscreenSrc ?? "").not.toContain("/media/");
  expect(await iframe.getAttribute("srcdoc")).toBeFalsy();
  await expect(frame.getByRole("heading", { name: "Live?" })).toBeVisible();

  await preview.getByTestId("file-preview-exit-fullscreen").click();
  await expect(preview).toHaveAttribute("data-fullscreen", "false");
  await expect(preview).toBeVisible();

  await header.getByTestId("file-preview-fullscreen").click();
  await expect(preview).toHaveAttribute("data-fullscreen", "true");
  await page.keyboard.press("Escape");
  await expect(preview).toHaveAttribute("data-fullscreen", "false");
  await expect(preview).toBeVisible();

  const xssAfterFullscreen = await page.evaluate(
    () => (window as Window & { __XSS__?: number }).__XSS__,
  );
  expect(xssAfterFullscreen).toBeUndefined();

  await preview.getByTestId("file-preview-html-tab-source").click();
  const source = preview.getByTestId("file-preview-source");
  await expect(source).toBeVisible();
  await expect(source).toContainText("window.__XSS__=1");
  await expect(header.getByTestId("file-preview-fullscreen")).toHaveCount(0);

  await expect(
    preview
      .getByTestId("file-preview-footer")
      .getByRole("button", { name: "Close" }),
  ).toBeVisible();
  await preview.getByTestId("file-preview-download").click();
  await expect.poll(() => e2eCommands(page)).toContain("download_file");
});

test("files above 10 MiB stay download-only; 3 MiB markdown still previews", async ({
  page,
}) => {
  const hugeUrl = `https://mock.relay/media/${"3".repeat(64)}.md`;
  const midUrl = `https://mock.relay/media/${"4".repeat(64)}.md`;

  await page.route(midUrl, (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/markdown",
      body: "# Mid-size preview\n\nFits under 10 MiB.",
    }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", `[huge.md](${hugeUrl})`, [
    [
      "imeta",
      `url ${hugeUrl}`,
      "m text/markdown",
      `x ${"3".repeat(64)}`,
      `size ${10 * 1024 * 1024 + 1}`,
      "filename huge.md",
    ],
  ]);

  const hugeCard = page.getByTestId("file-card").filter({ hasText: "huge.md" });
  await expect(hugeCard).toBeVisible();
  await hugeCard.click();

  const preview = page.getByTestId("file-preview-dialog");
  await expect(preview).toBeVisible();
  await expect(preview.getByTestId("file-preview-unavailable")).toBeVisible();
  await expect(preview).toContainText("too large to preview");
  expect(await e2eCommands(page)).not.toContain("download_file");

  await preview.getByTestId("file-preview-download").click();
  await expect.poll(() => e2eCommands(page)).toContain("download_file");

  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);

  await emitMockMessage(page, "general", `[mid.md](${midUrl})`, [
    [
      "imeta",
      `url ${midUrl}`,
      "m text/markdown",
      `x ${"4".repeat(64)}`,
      `size ${3 * 1024 * 1024}`,
      "filename mid.md",
    ],
  ]);

  const midCard = page.getByTestId("file-card").filter({ hasText: "mid.md" });
  await expect(midCard).toBeVisible();
  await midCard.click();
  await expect(preview).toBeVisible();
  const markdown = preview.getByTestId("file-preview-markdown");
  await expect(markdown).toBeVisible();
  await expect(markdown).toContainText("Mid-size preview");
});
