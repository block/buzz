import { readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import type { VideoBatch } from "../../src/features/huddle/lib/screenShare";

// Generated with the production OpenH264 encoder, not images or fake decoders.
const VIDEO = JSON.parse(
  readFileSync(
    new URL("../fixtures/huddle-video.json", import.meta.url),
    "utf8",
  ),
) as VideoBatch[];
const CHANNEL = "11111111-1111-4111-8111-111111111111";
const PARENT = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const SESSION = "22222222-2222-4222-8222-222222222222";

async function openHuddle(page: Page, companion = true) {
  await installMockBridge(page, {
    windowLabel: companion ? `huddle-${CHANNEL}` : "main",
    huddle: {
      parentChannelId: PARENT,
      ephemeralChannelId: CHANNEL,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
      transcriptionEnabled: false,
    },
  });
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Share screen", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "huddle",
            kind: 24811,
            exactChannel: true,
          }) ?? false,
      ),
    )
    .toBe(true);
}

async function streamVideo(page: Page) {
  await page.evaluate(
    ({ packets, session, pubkey }) => {
      const w = window as typeof window & {
        __VIDEO_TIMER__?: number;
        __VIDEO_INDEX__?: number;
      };
      const share = {
        session_id: session,
        started_at: Date.now() - 1000,
        label: "Project window",
      };
      let index = 0;
      const send = () => {
        const packet = packets[index % packets.length];
        const cycle = Math.floor(index / packets.length);
        const batch = {
          ...packet,
          share,
          sequence: index + 1,
          frames: packet.frames.map((frame) => ({
            ...frame,
            timestamp: frame.timestamp + cycle * 6_000_000,
          })),
        };
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "huddle",
          pubkey,
          kind: 24811,
          content: JSON.stringify(batch),
        });
        w.__VIDEO_INDEX__ = ++index;
      };
      send();
      w.__VIDEO_TIMER__ = window.setInterval(send, (8 * 1000) / 30);
    },
    { packets: VIDEO, session: SESSION, pubkey: TEST_IDENTITIES.tyler.pubkey },
  );
}

async function stopVideo(page: Page, notify = true) {
  await page.evaluate(
    ({ session, pubkey, notify }) => {
      const w = window as typeof window & { __VIDEO_TIMER__?: number };
      window.clearInterval(w.__VIDEO_TIMER__);
      if (notify)
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "huddle",
          pubkey,
          kind: 24811,
          content: JSON.stringify({
            type: "stopped",
            share: {
              session_id: session,
              started_at: Date.now() - 1000,
              label: "Project window",
            },
          }),
        });
    },
    { session: SESSION, pubkey: TEST_IDENTITIES.tyler.pubkey, notify },
  );
}

async function waitForVideo(page: Page) {
  const video = page.getByTestId("huddle-shared-screen").first();
  await expect(video).toBeVisible();
  await expect
    .poll(
      () =>
        video.evaluate((node) => {
          const video = node as HTMLVideoElement;
          return (
            video.videoWidth === 320 &&
            video.videoHeight === 180 &&
            video.readyState >= 2 &&
            !video.paused
          );
        }),
      { timeout: 15_000 },
    )
    .toBe(true);
  return video;
}

test("H.264 playback survives repeated buffer cleanup beyond 30 seconds", async ({
  page,
}) => {
  test.setTimeout(100_000);
  await openHuddle(page);
  await streamVideo(page);
  const video = await waitForVideo(page);
  let previous = await video.evaluate((node) => ({
    time: (node as HTMLVideoElement).currentTime,
    frames: (node as HTMLVideoElement).getVideoPlaybackQuality()
      .totalVideoFrames,
  }));
  for (let sample = 0; sample < 10; sample++) {
    await page.waitForTimeout(7500);
    const health = await video.evaluate((node) => {
      const video = node as HTMLVideoElement;
      return {
        error: video.error?.message ?? null,
        time: video.currentTime,
        frames: video.getVideoPlaybackQuality().totalVideoFrames,
        buffered: video.buffered.length
          ? video.buffered.end(video.buffered.length - 1) -
            video.buffered.start(0)
          : 0,
      };
    });
    expect(health.error).toBeNull();
    expect(health.time - previous.time).toBeGreaterThan(5);
    expect(health.frames - previous.frames).toBeGreaterThan(150);
    expect(health.buffered).toBeLessThan(45);
    await expect(page.getByRole("alert")).toHaveCount(0);
    previous = health;
  }
  await stopVideo(page);
});

test("keyboard source selection and stop use the native consent/state boundary", async ({
  page,
}) => {
  await openHuddle(page);
  await page.evaluate(() => {
    const w = window as typeof window & {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload?: Record<string, unknown>,
          options?: unknown,
        ) => Promise<unknown>;
      };
    };
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      if (command === "list_huddle_screen_sources")
        return [
          { id: "display:1", label: "Display 1", source_type: "display" },
          { id: "window:2", label: "Editor: project", source_type: "window" },
        ];
      if (
        command === "start_huddle_screen_share" ||
        command === "stop_huddle_screen_share"
      ) {
        const state = (await original("get_huddle_state")) as Record<
          string,
          unknown
        >;
        const share =
          command === "start_huddle_screen_share"
            ? {
                session_id: "22222222-2222-4222-8222-222222222222",
                started_at: Date.now(),
                label: String(payload?.sourceId),
              }
            : null;
        await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("huddle-state-changed", {
          ...state,
          screen_share: share,
        });
        return null;
      }
      return original(command, payload, options);
    };
  });
  await page
    .getByRole("button", { name: "Share screen", exact: true })
    .press("Enter");
  const chooser = page.getByRole("dialog", { name: "Share Screen" });
  await expect(
    chooser.getByRole("button", { name: "Share", exact: true }),
  ).toBeDisabled();
  await chooser.getByRole("radio", { name: "Editor: project" }).focus();
  await chooser.getByRole("radio", { name: "Editor: project" }).press("Space");
  await chooser
    .getByRole("button", { name: "Share", exact: true })
    .press("Enter");
  await expect(chooser).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop sharing screen" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Stop sharing screen" }).click();
  await expect(
    page.getByRole("button", { name: "Share screen", exact: true }),
  ).toBeVisible();
});

test("H.264 plays as moving video above 20 FPS and expansion retains live playback", async ({
  page,
}) => {
  await openHuddle(page);
  await streamVideo(page);
  const video = await waitForVideo(page);
  const before = await video.evaluate(
    (node) =>
      (node as HTMLVideoElement).getVideoPlaybackQuality().totalVideoFrames -
      (node as HTMLVideoElement).getVideoPlaybackQuality().droppedVideoFrames,
  );
  // A real-time observation interval distinguishes 30 FPS playback from packet-rate redraws.
  await page.waitForTimeout(1200);
  const after = await video.evaluate(
    (node) =>
      (node as HTMLVideoElement).getVideoPlaybackQuality().totalVideoFrames -
      (node as HTMLVideoElement).getVideoPlaybackQuality().droppedVideoFrames,
  );
  expect(after - before).toBeGreaterThan(24);
  console.log(`H.264 playback: ${after - before} presented frames in 1200ms`);
  const firstPixels = await video.evaluate((node) => {
    const canvas = document.createElement("canvas");
    canvas.width = 320;
    canvas.height = 180;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas unavailable");
    context.drawImage(node as HTMLVideoElement, 0, 0);
    return [...context.getImageData(0, 0, 320, 180).data].reduce(
      (sum, byte, index) => sum + byte * (index % 31),
      0,
    );
  });
  await page.waitForTimeout(300);
  const secondPixels = await video.evaluate((node) => {
    const canvas = document.createElement("canvas");
    canvas.width = 320;
    canvas.height = 180;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas unavailable");
    context.drawImage(node as HTMLVideoElement, 0, 0);
    return [...context.getImageData(0, 0, 320, 180).data].reduce(
      (sum, byte, index) => sum + byte * (index % 31),
      0,
    );
  });
  expect(secondPixels).not.toBe(firstPixels);
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/screenshots/huddle-video-stream.png",
  });
  await page.getByRole("button", { name: "Expand shared screen" }).click();
  const dialog = page.getByRole("dialog");
  await expect
    .poll(
      () =>
        dialog
          .getByTestId("huddle-shared-screen")
          .evaluate((node) => (node as HTMLVideoElement).readyState >= 2),
      { timeout: 10_000 },
    )
    .toBe(true);
  await dialog.getByRole("button", { name: "Collapse shared screen" }).click();
  await stopVideo(page);
  await expect(page.getByTestId("huddle-shared-screen")).toHaveCount(0);
});

test("a disconnected video producer expires and destroys its player", async ({
  page,
}) => {
  await openHuddle(page);
  await streamVideo(page);
  await waitForVideo(page);
  await stopVideo(page, false);
  await expect(page.getByTestId("huddle-shared-screen")).toHaveCount(0, {
    timeout: 7000,
  });
});

test("unsupported video stays terminal until an explicit playback retry", async ({
  page,
}) => {
  await openHuddle(page);
  await page.evaluate(() => {
    const original = MediaSource.isTypeSupported.bind(MediaSource);
    (
      window as typeof window & { __RESTORE_VIDEO_SUPPORT__?: () => void }
    ).__RESTORE_VIDEO_SUPPORT__ = () => {
      MediaSource.isTypeSupported = original;
    };
    MediaSource.isTypeSupported = () => false;
  });
  await streamVideo(page);
  await expect(page.getByRole("alert")).toHaveText(
    "This webview cannot decode the shared H.264 video.",
  );
  await page.evaluate(() => {
    (
      window as typeof window & { __RESTORE_VIDEO_SUPPORT__?: () => void }
    ).__RESTORE_VIDEO_SUPPORT__?.();
  });
  await page.waitForTimeout(1200);
  await expect(page.getByRole("alert")).toBeVisible();
  await page
    .getByRole("button", { name: "Retry video playback" })
    .press("Enter");
  await waitForVideo(page);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await stopVideo(page);
});

test("minimum native window keeps video and controls in view", async ({
  page,
}) => {
  await page.setViewportSize({ width: 720, height: 520 });
  await openHuddle(page);
  await streamVideo(page);
  await waitForVideo(page);
  const boxes = [];
  for (const name of [
    "Audio settings",
    "Speaker settings",
    "Share screen",
    "Emoji reactions",
    "Start transcript",
    "Add agent to huddle",
    "Return huddle to drawer",
    "Leave huddle",
  ]) {
    const button = page.getByRole("button", { name, exact: true });
    await expect(button).toBeInViewport();
    const box = await button.boundingBox();
    expect(box).not.toBeNull();
    if (box) boxes.push(box);
  }
  for (let i = 0; i < boxes.length; i++)
    for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i],
        b = boxes[j];
      expect(
        a.x + a.width <= b.x ||
          b.x + b.width <= a.x ||
          a.y + a.height <= b.y ||
          b.y + b.height <= a.y,
      ).toBe(true);
    }
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/screenshots/huddle-video-stream-minimum.png",
  });
  await stopVideo(page);
});

test("main window drains encoded native batches and retires the producer on stop", async ({
  page,
}) => {
  await openHuddle(page, false);
  await page.evaluate(
    ({ channel, session, packets }) => {
      const w = window as typeof window & {
        __VIDEO_READS__?: number;
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            payload?: Record<string, unknown>,
            options?: unknown,
          ) => Promise<unknown>;
        };
      };
      const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
      let share: {
        session_id: string;
        started_at: number;
        label: string;
      } | null = null;
      w.__VIDEO_READS__ = 0;
      w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
        if (command === "list_huddle_screen_sources")
          return [
            { id: "display:1", label: "Display 1", source_type: "display" },
          ];
        if (command === "get_huddle_state")
          return {
            ...((await original(command)) as Record<string, unknown>),
            screen_share: share,
          };
        if (
          command === "start_huddle_screen_share" ||
          command === "stop_huddle_screen_share"
        ) {
          if (command === "start_huddle_screen_share") {
            if (payload?.channelId !== channel)
              throw new Error("Wrong huddle scope");
            share = {
              session_id: session,
              started_at: Date.now(),
              label: "Display 1",
            };
          } else {
            share = null;
          }
          await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.(
            "huddle-state-changed",
            await w.__TAURI_INTERNALS__.invoke("get_huddle_state"),
          );
          return null;
        }
        if (command === "read_huddle_screen_video_chunk") {
          await new Promise((resolve) =>
            window.setTimeout(resolve, (8 * 1000) / 30),
          );
          if (!share) return null;
          const index = w.__VIDEO_READS__ ?? 0;
          w.__VIDEO_READS__ = index + 1;
          const packet = packets[index % packets.length];
          const cycle = Math.floor(index / packets.length);
          return original("sign_event", {
            kind: 24811,
            tags: [["h", channel]],
            content: JSON.stringify({
              ...packet,
              share,
              sequence: index + 1,
              frames: packet.frames.map((frame) => ({
                ...frame,
                timestamp: frame.timestamp + cycle * 6_000_000,
              })),
            }),
          });
        }
        return original(command, payload, options);
      };
    },
    { channel: CHANNEL, session: SESSION, packets: VIDEO },
  );
  await page.getByRole("button", { name: "Share screen", exact: true }).click();
  const chooser = page.getByRole("dialog", { name: "Share Screen" });
  await chooser.getByRole("radio", { name: "Display 1" }).check();
  await chooser.getByRole("button", { name: "Share", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "View shared screen" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __VIDEO_READS__?: number })
            .__VIDEO_READS__ ?? 0,
      ),
    )
    .toBeGreaterThanOrEqual(2);
  await page.getByRole("button", { name: "Stop sharing screen" }).click();
  await expect(
    page.getByRole("button", { name: "View shared screen" }),
  ).toHaveCount(0);
  const reads = await page.evaluate(
    () =>
      (window as typeof window & { __VIDEO_READS__?: number }).__VIDEO_READS__,
  );
  await page.waitForTimeout(600);
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __VIDEO_READS__?: number })
          .__VIDEO_READS__,
    ),
  ).toBe(reads);
});
