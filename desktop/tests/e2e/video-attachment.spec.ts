import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { expectCornerRadiusPx, expectSmoothCorners } from "../helpers/css";

const VIDEO_SHA = "b".repeat(64);
const VIDEO_URL = `http://localhost:3000/media/${VIDEO_SHA}.mp4`;
const PORTRAIT_VIDEO_SHA = "c".repeat(64);
const PORTRAIT_VIDEO_URL = `http://localhost:3000/media/${PORTRAIT_VIDEO_SHA}.mp4`;
const CONSTRAINED_LANDSCAPE_VIDEO_SHA = "d".repeat(64);
const CONSTRAINED_LANDSCAPE_VIDEO_URL = `http://localhost:3000/media/${CONSTRAINED_LANDSCAPE_VIDEO_SHA}.mp4`;
// Distinct-selector context-menu probes. The relay video is on the resolved
// relay origin (localhost:3000) so it is Download-eligible once the origin
// resolves; the off-relay video shares an identical /media/ path shape on a
// different origin, so it renders and offers Copy link but never Download.
const MENU_RELAY_VIDEO_SHA = "e".repeat(64);
const MENU_RELAY_VIDEO_URL = `http://localhost:3000/media/${MENU_RELAY_VIDEO_SHA}.mp4`;
const MENU_OFF_RELAY_VIDEO_SHA = "f".repeat(64);
const MENU_OFF_RELAY_VIDEO_URL = `https://cdn.example.com/media/${MENU_OFF_RELAY_VIDEO_SHA}.mp4`;
const VIDEO_REVIEW_NEUTRAL_ACCENT = "neutral";
const VIDEO_REVIEW_LIGHT_THEME = "catppuccin-latte";
// The fresh-profile default is the Buzz theme, which pins the neutral accent
// regardless of the stored accent color. Accent-driven review foreground
// assertions must run on a non-Buzz theme for the seeded accent to apply.
const VIDEO_REVIEW_ACCENT_THEME = "houston";
const VIDEO_REVIEW_ACCENT = "#ec4899";
const VIDEO_REVIEW_ACCENT_FOREGROUND_RGB = "rgb(240, 115, 177)";
const VIDEO_REVIEW_INDIGO_ACCENT = "#6366f1";
const VIDEO_REVIEW_INDIGO_FOREGROUND_RGB = "rgb(141, 143, 245)";
const VIDEO_REVIEW_NEUTRAL_DARK_RGB = "rgb(250, 250, 250)";
const POSTER_DATA_URL =
  "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxNjAgODAiPjxyZWN0IHdpZHRoPSIxNjAiIGhlaWdodD0iODAiIGZpbGw9IiMyNjQ2NTMiLz48Y2lyY2xlIGN4PSI1NCIgY3k9IjQwIiByPSIyMiIgZmlsbD0iI2YyYzE0ZSIvPjxwYXRoIGQ9Ik05MiAyNGg0NHYzMkg5MnoiIGZpbGw9IiNmNzgxNTQiLz48L3N2Zz4=";

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(async () => {
      return page.evaluate((channelName) => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }) ?? false
        );
      }, channelName);
    })
    .toBe(true);
}

function emitMockMessage(
  page: Page,
  channelName: string,
  content: string,
  options: { extraTags?: string[][] } = {},
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
      emit({ channelName, content, extraTags });
    },
    { channelName, content, extraTags: options.extraTags },
  );
}

async function installVideoReviewHarness(
  page: Page,
  {
    accentColor = VIDEO_REVIEW_ACCENT,
    themeName,
  }: { accentColor?: string; themeName?: string } = {},
) {
  await page.addInitScript(
    ({ accentColor, themeName }) => {
      if (themeName) {
        window.localStorage.setItem("buzz-theme", themeName);
      }
      window.localStorage.setItem("buzz-accent-color", accentColor);

      type MediaState = {
        currentTime: number;
        paused: boolean;
      };
      const mediaState = new WeakMap<HTMLMediaElement, MediaState>();
      const getMediaState = (element: HTMLMediaElement) => {
        let state = mediaState.get(element);
        if (!state) {
          state = { currentTime: 0, paused: true };
          mediaState.set(element, state);
        }
        return state;
      };

      Object.defineProperty(HTMLMediaElement.prototype, "load", {
        configurable: true,
        value() {
          getMediaState(this as HTMLMediaElement).currentTime = 0;
        },
      });
      Object.defineProperty(HTMLMediaElement.prototype, "play", {
        configurable: true,
        value() {
          getMediaState(this as HTMLMediaElement).paused = false;
          this.dispatchEvent(new Event("play"));
          return Promise.resolve();
        },
      });
      Object.defineProperty(HTMLMediaElement.prototype, "pause", {
        configurable: true,
        value() {
          getMediaState(this as HTMLMediaElement).paused = true;
          this.dispatchEvent(new Event("pause"));
        },
      });
      Object.defineProperty(HTMLMediaElement.prototype, "paused", {
        configurable: true,
        get() {
          return getMediaState(this as HTMLMediaElement).paused;
        },
      });
      Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
        configurable: true,
        get() {
          return getMediaState(this as HTMLMediaElement).currentTime;
        },
        set(value) {
          getMediaState(this as HTMLMediaElement).currentTime =
            Number(value) || 0;
          this.dispatchEvent(new Event("seeked"));
        },
      });
      Object.defineProperty(HTMLMediaElement.prototype, "duration", {
        configurable: true,
        get() {
          return 12.5;
        },
      });
    },
    { accentColor, themeName },
  );

  await installMockBridge(page, {
    uploadDescriptors: [
      {
        url: VIDEO_URL,
        sha256: VIDEO_SHA,
        size: 987_654,
        type: "video/mp4",
        uploaded: Math.floor(Date.now() / 1000),
        duration: 12.5,
        image: POSTER_DATA_URL,
        dim: "160x80",
        filename: "launch-demo.mp4",
      },
    ],
  });
}

async function openReviewWithPostedTimecode(
  page: Page,
  commentText = "Neutral accent check",
) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.getByRole("button", { name: "Attach image" }).click();
  await expect(
    page.getByTestId("message-composer").getByAltText("Video attachment bbbb"),
  ).toBeVisible();
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const reviewButton = page
    .getByRole("button", { name: "Open video review" })
    .last();
  await expect(reviewButton).toBeVisible();
  await page.waitForFunction(() => {
    const launcher = document.querySelector("[data-video-review-launcher]");
    const row = launcher?.closest("[data-message-id]");
    const messageId = row?.getAttribute("data-message-id") ?? "";
    return Boolean(messageId) && !messageId.startsWith("optimistic");
  });
  await reviewButton.evaluate((button) =>
    (button as HTMLButtonElement).click(),
  );

  const reviewDialog = page.getByTestId("video-review-dialog");
  await expect(reviewDialog).toBeVisible();
  await reviewDialog.locator("video").evaluate((video) => {
    const el = video as HTMLVideoElement;
    el.currentTime = 10;
    el.dispatchEvent(new Event("timeupdate"));
  });
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );

  const commentBox = reviewDialog.getByTestId("message-input");
  await commentBox.click();
  await commentBox.fill(commentText);
  await reviewDialog.getByTestId("send-message").click();
  await expect(page.getByTestId("video-review-comments")).toContainText(
    commentText,
  );

  return reviewDialog;
}

test("video upload previews use poster frames and inline videos open review mode", async ({
  page,
}) => {
  await installVideoReviewHarness(page, {
    themeName: VIDEO_REVIEW_ACCENT_THEME,
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.getByRole("button", { name: "Attach image" }).click();

  const composer = page.getByTestId("message-composer");
  const composerPoster = composer.getByAltText("Video attachment bbbb");
  await expect(composerPoster).toBeVisible();
  await expect(composerPoster).toHaveAttribute("src", POSTER_DATA_URL);

  const box = await composerPoster.boundingBox();
  expect(box?.width).toBeGreaterThanOrEqual(53);
  expect(box?.width).toBeLessThanOrEqual(58);
  expect(box?.height).toBeGreaterThanOrEqual(52);
  expect(box?.height).toBeLessThanOrEqual(58);

  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const reviewButton = page
    .getByRole("button", { name: "Open video review" })
    .last();
  await expect(reviewButton).toBeVisible();
  await page.waitForFunction(() => {
    const launcher = document.querySelector("[data-video-review-launcher]");
    const row = launcher?.closest("[data-message-id]");
    const messageId = row?.getAttribute("data-message-id") ?? "";
    return Boolean(messageId) && !messageId.startsWith("optimistic");
  });
  const videoMessageId = await reviewButton.evaluate((button) =>
    button.closest("[data-message-id]")?.getAttribute("data-message-id"),
  );
  if (!videoMessageId) {
    throw new Error("Expected uploaded video row to have a message id.");
  }

  const inlinePlayer = page.getByTestId("video-player").last();
  const inlineSurface = inlinePlayer.locator("[data-smooth-corners]").first();
  await expectCornerRadiusPx(inlineSurface, 16);
  await expectSmoothCorners(inlineSurface);
  const inlineVideo = inlinePlayer.locator("video");
  await inlinePlayer.getByRole("button", { name: "Play video" }).click();

  // Inline playback uses our own controls — the native browser UI must
  // never appear.
  await expect(inlineVideo).not.toHaveAttribute("controls", "");
  await expect(
    inlinePlayer.getByRole("button", { name: "Pause video" }),
  ).toBeVisible();
  await expect(page.getByTestId("video-inline-time")).toHaveText("00:00");
  await expect(page.getByTestId("video-inline-duration")).toHaveText("00:12");

  // Scrub the inline timeline to the middle.
  const inlineTrack = page.getByTestId("video-inline-progress-track");
  const inlineTrackBox = await inlineTrack.boundingBox();
  expect(inlineTrackBox).not.toBeNull();
  if (!inlineTrackBox) {
    throw new Error("Expected the inline timeline track to have a box");
  }
  await page.mouse.click(
    inlineTrackBox.x + inlineTrackBox.width * 0.5,
    inlineTrackBox.y + inlineTrackBox.height / 2,
  );
  await expect
    .poll(() =>
      inlineVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeGreaterThan(6);
  await expect
    .poll(() =>
      inlineVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeLessThan(6.5);
  await expect(page.getByTestId("video-inline-time")).toHaveText("00:06");

  await inlinePlayer.getByRole("button", { name: "Pause video" }).click();
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const restoredInlinePlayer = page.getByTestId("video-player").last();
  const restoredInlineVideo = restoredInlinePlayer.locator("video");
  await restoredInlineVideo.evaluate((video) => {
    video.dispatchEvent(new Event("loadedmetadata"));
  });
  await expect
    .poll(() =>
      restoredInlineVideo.evaluate(
        (video) => (video as HTMLVideoElement).currentTime,
      ),
    )
    .toBeGreaterThan(6);
  await expect
    .poll(() =>
      restoredInlineVideo.evaluate(
        (video) => (video as HTMLVideoElement).currentTime,
      ),
    )
    .toBeLessThan(6.5);
  await restoredInlinePlayer
    .getByRole("button", { name: "Play video" })
    .click();
  await expect(
    restoredInlinePlayer.getByRole("button", { name: "Pause video" }),
  ).toBeVisible();

  const inlineSpeedButton =
    restoredInlinePlayer.getByTestId("video-inline-speed");
  await expect(inlineSpeedButton).toHaveText("1x");
  await inlineSpeedButton.click();
  const inlineSpeedMenu = page.getByTestId("video-inline-speed-menu");
  await expect(inlineSpeedMenu.getByRole("button")).toHaveText([
    "2x",
    "1.75x",
    "1.5x",
    "1.25x",
    "1x",
    "0.75x",
    "0.5x",
    "0.25x",
  ]);
  await inlineSpeedMenu
    .getByRole("button", { name: "1.5x", exact: true })
    .click();
  await expect(inlineSpeedButton).toHaveText("1.5x");
  await expect
    .poll(() =>
      restoredInlineVideo.evaluate(
        (video) => (video as HTMLVideoElement).playbackRate,
      ),
    )
    .toBe(1.5);

  // Inline volume controls.
  await inlinePlayer.getByRole("button", { name: "Mute" }).click();
  await expect
    .poll(() =>
      inlineVideo.evaluate((video) => (video as HTMLVideoElement).muted),
    )
    .toBe(true);
  await inlinePlayer.getByRole("button", { name: "Unmute" }).click();
  await expect
    .poll(() =>
      inlineVideo.evaluate((video) => (video as HTMLVideoElement).muted),
    )
    .toBe(false);

  // Reset the inline position so the review dialog opens from the start.
  await inlinePlayer.getByRole("button", { name: "Pause video" }).click();
  await inlineVideo.evaluate((video) => {
    (video as HTMLVideoElement).currentTime = 0;
  });

  await reviewButton.evaluate((button) =>
    (button as HTMLButtonElement).click(),
  );

  const reviewDialog = page.getByTestId("video-review-dialog");
  await expect(reviewDialog).toBeVisible();
  await expect(reviewDialog.getByText("launch-demo.mp4")).toBeVisible();
  const reviewBox = await reviewDialog.boundingBox();
  const viewport = page.viewportSize();
  expect(reviewBox?.x).toBeGreaterThan(0);
  expect(reviewBox?.y).toBeGreaterThan(0);
  expect(reviewBox?.width).toBeLessThan(viewport?.width ?? 0);
  expect(reviewBox?.height).toBeLessThan(viewport?.height ?? 0);
  await expect(
    reviewDialog.getByRole("button", { name: "Play review video" }),
  ).toBeVisible();
  await expect(reviewDialog.getByLabel("Video timeline")).toBeVisible();

  const reviewVideo = reviewDialog.locator("video");
  await expect(reviewVideo).not.toHaveAttribute("controls", "");
  const reviewSpeedButton = reviewDialog.getByTestId("video-review-speed");
  await expect(reviewSpeedButton).toHaveText("1.5x");
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).playbackRate),
    )
    .toBe(1.5);
  await reviewSpeedButton.click();
  await page
    .getByTestId("video-review-speed-menu")
    .getByRole("button", { name: "0.25x", exact: true })
    .click();
  await expect(reviewSpeedButton).toHaveText("0.25x");
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).playbackRate),
    )
    .toBe(0.25);
  await reviewDialog.getByRole("button", { name: "Play review video" }).click();
  await expect(
    reviewDialog.getByRole("button", { name: "Pause review video" }),
  ).toBeVisible();
  const progressThumb = page.getByTestId("video-review-progress-thumb");
  await reviewVideo.evaluate((video) => {
    const el = video as HTMLVideoElement;
    el.currentTime = 10;
  });
  await reviewDialog
    .getByRole("button", { name: "Pause review video" })
    .click();
  await expect(
    reviewDialog.getByRole("button", { name: "Play review video" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBe(10);
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );
  await expect(page.getByTestId("video-review-time-current")).toHaveText(
    "00:10",
  );
  await expect(page.getByTestId("video-review-time-duration")).toHaveText(
    "00:12",
  );
  const progressTrack = page.getByTestId("video-review-progress-track");
  await expect
    .poll(() =>
      progressTrack.evaluate(
        (el) => window.getComputedStyle(el).backgroundColor,
      ),
    )
    .toContain("0.16");
  await expect
    .poll(() =>
      progressThumb.evaluate((el) => window.getComputedStyle(el).opacity),
    )
    .toBe("1");
  const timeline = page.getByTestId("video-review-timeline");
  await expect(timeline).toBeVisible();
  const trackBox = await progressTrack.boundingBox();
  expect(trackBox).not.toBeNull();
  if (!trackBox) {
    throw new Error(
      "Expected the review timeline track to have a bounding box",
    );
  }
  expect(trackBox.width).toBeGreaterThan(0);

  const timelineY = trackBox.y + trackBox.height / 2;
  await page.mouse.click(trackBox.x + trackBox.width * 0.5, timelineY);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeGreaterThan(6);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeLessThan(6.5);
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:06",
  );

  await page.mouse.move(trackBox.x + trackBox.width * 0.2, timelineY);
  await page.mouse.down();
  await page.mouse.move(trackBox.x + trackBox.width * 0.8, timelineY);
  await page.mouse.up();
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBe(10);
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );

  await page.mouse.move(trackBox.x + trackBox.width * 0.8, timelineY);
  await page.mouse.down();
  await page.mouse.move(trackBox.x + trackBox.width * 0.5, timelineY);
  await page.mouse.up();
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeGreaterThan(6);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeLessThan(6.5);
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:06",
  );

  await page.mouse.click(trackBox.x + trackBox.width * 0.8, timelineY);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBe(10);
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );

  await page.mouse.move(trackBox.x + trackBox.width * 0.5, timelineY);
  await expect
    .poll(() =>
      progressThumb.evaluate((el) => window.getComputedStyle(el).opacity),
    )
    .toBe("1");
  await expect(page.getByTestId("video-review-reaction-tray")).toBeVisible();
  await expect
    .poll(() =>
      page
        .getByTestId("video-review-progress-fill")
        .evaluate((el) => (el as HTMLElement).style.width),
    )
    .toBe("80%");

  await reviewDialog.getByRole("button", { name: "More reactions" }).click();
  await expect(page.getByTestId("video-review-emoji-picker")).toBeVisible();
  await reviewDialog.getByRole("button", { name: "More reactions" }).click();
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );
  await reviewDialog.getByTestId("video-review-reaction-0").click();
  await expect(page.getByTestId("video-review-comments")).toContainText("😂");

  await reviewVideo.evaluate((video) => {
    const el = video as HTMLVideoElement;
    el.currentTime = 10.7;
    el.dispatchEvent(new Event("timeupdate"));
  });
  await reviewDialog.getByTestId("video-review-reaction-1").click();
  const fractionalTimecode = reviewDialog
    .getByRole("button", { name: "Jump to 00:10.7" })
    .first();
  await expect(fractionalTimecode).toBeVisible();

  await page.mouse.click(trackBox.x + trackBox.width * 0.5, timelineY);
  await fractionalTimecode.click();
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeGreaterThan(10.65);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeLessThan(10.75);

  await reviewVideo.evaluate((video) => {
    const el = video as HTMLVideoElement;
    el.currentTime = 10;
    el.dispatchEvent(new Event("timeupdate"));
  });
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );

  // The comments panel embeds the standard message composer.
  const reviewComposer = reviewDialog.getByTestId("message-composer");
  await expect(reviewComposer).toBeVisible();
  const commentBox = reviewDialog.getByTestId("message-input");
  await commentBox.click();
  await expect(commentBox).toBeFocused();
  await commentBox.fill("Color pass looks right");
  await expect(commentBox).toHaveText("Color pass looks right");
  await reviewDialog.getByTestId("send-message").click();

  await expect(page.getByTestId("video-review-comments")).toContainText(
    "Color pass looks right",
  );
  await expect(
    reviewDialog.getByRole("button", { name: "Seek to 00:10" }).first(),
  ).toBeVisible();
  const commentTimecode = reviewDialog
    .getByRole("button", { name: "Jump to 00:10" })
    .first();
  await expect(commentTimecode).toBeVisible();
  await expect(
    reviewDialog.getByTestId("video-review-comment-timecode").first(),
  ).toHaveText("00:10");
  await expect(
    reviewDialog.getByTestId("video-review-comment-timecode").first(),
  ).toHaveCSS("color", VIDEO_REVIEW_ACCENT_FOREGROUND_RGB);

  // Regression: a timeline re-render (live message arriving in the channel)
  // must not remount the review dialog or wipe an in-progress comment draft.
  await commentBox.click();
  await commentBox.fill("Second pass note");
  await emitMockMessage(page, "general", "Unrelated chatter mid-review");
  await expect(
    page
      .getByTestId("message-row")
      .filter({ hasText: "Unrelated chatter mid-review" }),
  ).toHaveCount(1);
  await expect(commentBox).toHaveText("Second pass note");
  await expect(commentBox).toBeFocused();
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );
  await commentBox.fill("");

  await page.mouse.click(trackBox.x + trackBox.width * 0.5, timelineY);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeGreaterThan(6);
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBeLessThan(6.5);

  await commentTimecode.click();
  await expect
    .poll(() =>
      reviewVideo.evaluate((video) => (video as HTMLVideoElement).currentTime),
    )
    .toBe(10);
  await expect(page.getByTestId("video-review-composer-timecode")).toHaveText(
    "00:10",
  );
  await expect(page.getByTestId("video-review-time-current")).toHaveText(
    "00:10",
  );
  await expect(page.getByTestId("video-review-time-duration")).toHaveText(
    "00:12",
  );

  // Reply to the text comment: posts nested under it, with no new timecode.
  const textCommentCard = page
    .getByTestId("video-review-comments")
    .locator("article")
    .filter({ hasText: "Color pass looks right" });
  await textCommentCard
    .getByTestId("video-review-comment-reply")
    .first()
    .click();
  await expect(commentBox).toBeFocused();
  await expect(reviewDialog.getByTestId("reply-target")).toBeVisible();
  await commentBox.fill("Agreed, shipping it");
  await reviewDialog.getByTestId("send-message").click();
  await expect(textCommentCard).toContainText("Agreed, shipping it");
  await expect(reviewDialog.getByTestId("reply-target")).toHaveCount(0);
  const replyTimecodeChips = textCommentCard.getByTestId(
    "video-review-comment-timecode",
  );
  await expect(replyTimecodeChips).toHaveCount(1);

  // Frame-stamp toggle off → the comment posts without a timecode chip and
  // the composer chip drops to its muted state.
  const frameToggle = reviewDialog.getByTestId("video-review-frame-toggle");
  await expect(frameToggle).toHaveAttribute("data-state", "checked");
  await frameToggle.click();
  await expect(frameToggle).toHaveAttribute("data-state", "unchecked");
  await commentBox.fill("Untimed general note");
  await reviewDialog.getByTestId("send-message").click();
  const untimedCard = page
    .getByTestId("video-review-comments")
    .locator("article")
    .filter({ hasText: "Untimed general note" });
  await expect(untimedCard).toBeVisible();
  await expect(
    untimedCard.getByTestId("video-review-comment-timecode"),
  ).toHaveCount(0);
  await frameToggle.click();
  await expect(frameToggle).toHaveAttribute("data-state", "checked");

  // The comments panel collapses and reopens (animated width).
  const commentsPanel = page.getByTestId("video-review-comments-panel");
  await reviewDialog.getByTestId("video-review-toggle-comments").click();
  await expect
    .poll(() =>
      commentsPanel.evaluate((el) => el.getBoundingClientRect().width),
    )
    .toBe(0);
  await expect(commentsPanel).toHaveAttribute("inert", "");
  await reviewDialog.getByTestId("video-review-toggle-comments").click();
  await expect
    .poll(() =>
      commentsPanel.evaluate((el) => el.getBoundingClientRect().width),
    )
    .toBe(380);
  await expect(commentsPanel).not.toHaveAttribute("inert", "");
  await expect(reviewDialog.getByTestId("message-composer")).toBeVisible();

  await reviewDialog
    .getByRole("button", { name: "Close video review" })
    .click();
  await expect(page.getByTestId("video-review-dialog")).toHaveCount(0);
  await expect(
    restoredInlinePlayer.getByTestId("video-inline-speed"),
  ).toHaveText("0.25x");
  await expect
    .poll(() =>
      restoredInlineVideo.evaluate(
        (video) => (video as HTMLVideoElement).playbackRate,
      ),
    )
    .toBe(0.25);

  await reviewButton.evaluate((button) =>
    (button as HTMLButtonElement).click(),
  );
  await expect(page.getByTestId("video-review-dialog")).toBeVisible();
  await page
    .getByTestId("video-review-backdrop")
    .click({ position: { x: 4, y: 4 } });
  await expect(page.getByTestId("video-review-dialog")).toHaveCount(0);

  const videoSummaryRow = page.locator(
    `[data-thread-head-id="${videoMessageId}"]`,
  );
  await expect(videoSummaryRow).toBeVisible();
  await videoSummaryRow.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const threadHead = threadPanel.getByTestId("message-thread-head");
  await expect(threadHead.getByTestId("video-player")).toBeVisible();

  await threadHead.getByRole("button", { name: "Open video review" }).click();
  const threadReviewDialog = page.getByTestId("video-review-dialog");
  await expect(threadReviewDialog).toBeVisible();
  await expect(
    threadReviewDialog.getByTestId("video-review-comments-panel"),
  ).toBeVisible();
  await expect(
    threadReviewDialog.getByTestId("message-composer"),
  ).toBeVisible();
  await expect(
    threadReviewDialog.getByTestId("video-review-comments"),
  ).toContainText("Color pass looks right");
});

test("narrow inline videos hide playback speed control", async ({ page }) => {
  await installVideoReviewHarness(page);

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", `![video](${PORTRAIT_VIDEO_URL})`, {
    extraTags: [
      [
        "imeta",
        `url ${PORTRAIT_VIDEO_URL}`,
        "m video/mp4",
        `x ${PORTRAIT_VIDEO_SHA}`,
        "size 987654",
        "dim 80x160",
        "duration 12.5",
        `image ${POSTER_DATA_URL}`,
        "filename portrait-demo.mp4",
      ],
    ],
  });

  const portraitPlayer = page.getByTestId("video-player").last();
  await expect(portraitPlayer).toBeVisible();
  const portraitBox = await portraitPlayer.boundingBox();
  expect(portraitBox?.width).toBeLessThan(220);

  await portraitPlayer.getByRole("button", { name: "Play video" }).click();
  await expect(
    portraitPlayer.getByTestId("video-inline-controls"),
  ).toBeVisible();
  await expect(portraitPlayer.getByTestId("video-inline-speed")).toHaveCount(0);

  await portraitPlayer
    .getByRole("button", { name: "Open video review" })
    .click();
  const reviewDialog = page.getByTestId("video-review-dialog");
  await expect(reviewDialog).toBeVisible();
  await expect(reviewDialog.getByTestId("video-review-speed")).toHaveText("1x");
});

test("constrained landscape inline videos measure rendered width before showing speed", async ({
  page,
}) => {
  await installVideoReviewHarness(page);

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(
    page,
    "general",
    `![video](${CONSTRAINED_LANDSCAPE_VIDEO_URL})`,
    {
      extraTags: [
        [
          "imeta",
          `url ${CONSTRAINED_LANDSCAPE_VIDEO_URL}`,
          "m video/mp4",
          `x ${CONSTRAINED_LANDSCAPE_VIDEO_SHA}`,
          "size 987654",
          "dim 160x80",
          "duration 12.5",
          `image ${POSTER_DATA_URL}`,
          "filename constrained-landscape-demo.mp4",
        ],
      ],
    },
  );

  const landscapePlayer = page.getByTestId("video-player").last();
  await expect(landscapePlayer).toBeVisible();
  const fullWidthBox = await landscapePlayer.boundingBox();
  expect(fullWidthBox?.width).toBeGreaterThan(220);

  await landscapePlayer.getByRole("button", { name: "Play video" }).click();
  await expect(
    landscapePlayer.getByTestId("video-inline-controls"),
  ).toBeVisible();
  await expect(landscapePlayer.getByTestId("video-inline-speed")).toBeVisible();

  await landscapePlayer.evaluate((element) => {
    (element as HTMLElement).style.width = "180px";
  });
  await expect
    .poll(async () => {
      const box = await landscapePlayer.boundingBox();
      return box?.width ?? 0;
    })
    .toBeLessThan(220);
  await expect(landscapePlayer.getByTestId("video-inline-speed")).toHaveCount(
    0,
  );
});

test("neutral accent uses the forced-dark review foreground", async ({
  page,
}) => {
  await installVideoReviewHarness(page, {
    accentColor: VIDEO_REVIEW_NEUTRAL_ACCENT,
    themeName: VIDEO_REVIEW_LIGHT_THEME,
  });

  const reviewDialog = await openReviewWithPostedTimecode(page);

  await expect(
    reviewDialog.getByTestId("video-review-composer-timecode"),
  ).toHaveCSS("color", VIDEO_REVIEW_NEUTRAL_DARK_RGB);
  await expect(
    reviewDialog.getByTestId("video-review-comment-timecode").first(),
  ).toHaveCSS("color", VIDEO_REVIEW_NEUTRAL_DARK_RGB);
});

test("dark accent uses a contrast-safe review foreground", async ({ page }) => {
  await installVideoReviewHarness(page, {
    accentColor: VIDEO_REVIEW_INDIGO_ACCENT,
    themeName: VIDEO_REVIEW_ACCENT_THEME,
  });

  const reviewDialog = await openReviewWithPostedTimecode(
    page,
    "Indigo contrast check",
  );

  await expect(
    reviewDialog.getByTestId("video-review-composer-timecode"),
  ).toHaveCSS("color", VIDEO_REVIEW_INDIGO_FOREGROUND_RGB);
  await expect(
    reviewDialog.getByTestId("video-review-comment-timecode").first(),
  ).toHaveCSS("color", VIDEO_REVIEW_INDIGO_FOREGROUND_RGB);
});

// Emit a video message with the imeta the renderer needs to pick the video
// player and, for the relay case, a resolvable download URL.
function emitVideoMessage(
  page: Page,
  { url, sha, filename }: { url: string; sha: string; filename: string },
) {
  return emitMockMessage(page, "general", `![video](${url})`, {
    extraTags: [
      [
        "imeta",
        `url ${url}`,
        "m video/mp4",
        `x ${sha}`,
        "size 987654",
        "dim 160x80",
        "duration 12.5",
        `image ${POSTER_DATA_URL}`,
        `filename ${filename}`,
      ],
    ],
  });
}

test("right-click menus expose distinct selectors for links, relay video, and off-relay video", async ({
  page,
}) => {
  await installVideoReviewHarness(page);

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  // ── Link menu: Open link + Copy link, never image/video attributes ────────
  await emitMockMessage(
    page,
    "general",
    "Docs at https://example.com/handbook",
  );
  const link = page.getByRole("link", { name: "https://example.com/handbook" });
  await expect(link).toBeVisible();
  await link.scrollIntoViewIfNeeded();
  // Fire a real bubbling `contextmenu` MouseEvent on the anchor rather than
  // `.click({ button: "right" })`: the link is wrapped in a hover tooltip and a
  // positional right-click can land on the tooltip layer (or merely select the
  // text) without reaching the anchor's React `onContextMenuCapture`. An
  // element-targeted bubbling event drives the capture handler deterministically.
  await link.evaluate((el) =>
    el.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
    ),
  );

  const linkMenu = page.locator("[data-link-context-menu]");
  await expect(linkMenu).toBeVisible();
  await expect(page.locator("[data-media-context-menu]")).toBeVisible();
  await expect(
    linkMenu.getByRole("button", { name: "Open link" }),
  ).toBeVisible();
  await expect(
    linkMenu.getByRole("button", { name: "Copy link" }),
  ).toBeVisible();
  await expect(page.locator("[data-image-context-menu]")).toHaveCount(0);
  await expect(page.locator("[data-video-context-menu]")).toHaveCount(0);
  // Dismiss the menu before the next probe: the menu closes on any click.
  await page.getByTestId("chat-title").click();
  await expect(linkMenu).toHaveCount(0);

  // ── Relay video menu: Download video + Copy link, appearing only once the
  // relay origin resolves (the reactivity fix) ─────────────────────────────
  await emitVideoMessage(page, {
    url: MENU_RELAY_VIDEO_URL,
    sha: MENU_RELAY_VIDEO_SHA,
    filename: "relay-clip.mp4",
  });
  const relayPlayer = page.getByTestId("video-player").last();
  await expect(relayPlayer).toBeVisible();
  // Right-click the player surface. `force` skips the actionability guard: the
  // Play-button overlay sits above the video, but the contextmenu event still
  // capture-bubbles to the surface handler that opens the menu.
  await relayPlayer.click({ button: "right", force: true });

  const videoMenu = page.locator("[data-video-context-menu]");
  await expect(videoMenu).toBeVisible();
  await expect(page.locator("[data-media-context-menu]")).toBeVisible();
  // Download eligibility is reactive: the relay origin resolves asynchronously
  // (commonly after first render), and when it does the already-open menu
  // recomputes to reveal Download — no re-navigation or menu re-open.
  await expect(
    videoMenu.getByRole("button", { name: "Download video" }),
  ).toBeVisible();
  await expect(
    videoMenu.getByRole("button", { name: "Copy link" }),
  ).toBeVisible();
  await expect(page.locator("[data-image-context-menu]")).toHaveCount(0);
  await expect(page.locator("[data-link-context-menu]")).toHaveCount(0);

  // The Download action drives the native download_file command.
  await videoMenu.getByRole("button", { name: "Download video" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("download_file");

  // Dismiss the relay menu before the off-relay probe so `[data-video-context
  // -menu]` refers unambiguously to the off-relay video's menu below. The menu
  // closes on any click.
  await page.getByTestId("chat-title").click();
  await expect(page.locator("[data-video-context-menu]")).toHaveCount(0);

  // ── Off-relay video control: renders and offers Copy link, never Download ─
  await emitVideoMessage(page, {
    url: MENU_OFF_RELAY_VIDEO_URL,
    sha: MENU_OFF_RELAY_VIDEO_SHA,
    filename: "external-clip.mp4",
  });
  const offRelayPlayer = page.getByTestId("video-player").last();
  await expect(offRelayPlayer).toBeVisible();
  await offRelayPlayer.click({ button: "right", force: true });

  const offRelayMenu = page.locator("[data-video-context-menu]");
  await expect(offRelayMenu).toBeVisible();
  await expect(
    offRelayMenu.getByRole("button", { name: "Copy link" }),
  ).toBeVisible();
  // An off-relay video would fail the download command's SSRF gate, so the
  // menu must never offer Download for it.
  await expect(
    offRelayMenu.getByRole("button", { name: "Download video" }),
  ).toHaveCount(0);
});
