import { readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import type { VideoBatch } from "../../src/features/huddle/lib/screenShare";

const CHANNEL = "11111111-1111-4111-8111-111111111111";
const PARENT = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const SESSION = "22222222-2222-4222-8222-222222222222";
const VIDEO = JSON.parse(
  readFileSync(
    new URL("../fixtures/huddle-video.json", import.meta.url),
    "utf8",
  ),
) as VideoBatch[];

async function open(page: Page, main = true) {
  await installMockBridge(page, {
    windowLabel: main ? "main" : `huddle-${CHANNEL}`,
    huddle: {
      parentChannelId: PARENT,
      ephemeralChannelId: CHANNEL,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
      transcriptionEnabled: false,
    },
  });
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Turn camera on", exact: true }),
  ).toBeVisible();
}

async function cameraNative(page: Page, throttleWindowTimers = false) {
  await page.evaluate(
    ({ channel, session, packets, throttleWindowTimers }) => {
      const w = window as typeof window & {
        __CAMERA_PUSHES__?: number;
        __CAMERA_BATCHES__?: number;
        __CAMERA_REQUESTS__?: MediaStreamConstraints[];
        __CAMERA_TRACK__?: MediaStreamTrack;
        __CAMERA_DRAW__?: number;
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            payload?: unknown,
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
      let index = 0;
      w.__CAMERA_PUSHES__ = 0;
      w.__CAMERA_BATCHES__ = 0;
      w.__CAMERA_REQUESTS__ = [];
      const nativeSetTimeout = window.setTimeout.bind(window);
      if (throttleWindowTimers) {
        window.setTimeout = (callback, delay = 0, ...args) =>
          nativeSetTimeout(
            callback,
            delay <= 34 ? Math.max(1000, delay) : delay,
            ...args,
          );
      }
      const nativeMedia = navigator.mediaDevices.getUserMedia.bind(
        navigator.mediaDevices,
      );
      navigator.mediaDevices.getUserMedia = async (constraints) => {
        if (!constraints.video) return nativeMedia(constraints);
        w.__CAMERA_REQUESTS__?.push(constraints);
        const canvas = document.createElement("canvas");
        canvas.width = 320;
        canvas.height = 180;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Canvas unavailable");
        let tick = 0;
        const draw = () => {
          context.fillStyle = "#145044";
          context.fillRect(0, 0, 320, 180);
          context.fillStyle = "#ff4055";
          context.fillRect((tick++ * 6) % 260, 40, 40, 100);
          w.__CAMERA_DRAW__ = nativeSetTimeout(draw, 33);
        };
        draw();
        const stream = canvas.captureStream(30);
        w.__CAMERA_TRACK__ = stream.getVideoTracks()[0];
        return stream;
      };
      w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
        const args = payload as Record<string, unknown>;
        if (command === "get_huddle_state")
          return {
            ...((await original(command)) as Record<string, unknown>),
            camera_share: share,
            camera_device_id: "",
            camera_error: null,
          };
        if (command === "start_huddle_camera") {
          if (args.channelId !== channel) throw new Error("Wrong camera room");
          share = {
            session_id: session,
            started_at: Date.now(),
            label: "Virtual camera",
          };
          await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.(
            "huddle-state-changed",
            await w.__TAURI_INTERNALS__.invoke("get_huddle_state"),
          );
          return share;
        }
        if (command === "stop_huddle_camera") {
          share = null;
          await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.(
            "huddle-state-changed",
            await w.__TAURI_INTERNALS__.invoke("get_huddle_state"),
          );
          return null;
        }
        if (command === "push_huddle_camera_frame") {
          if (!(payload instanceof Uint8Array))
            throw new Error("Camera must use raw binary IPC");
          const headers = (options as { headers?: Record<string, string> })
            .headers;
          if (headers?.["x-buzz-camera-session"] !== session)
            throw new Error("Wrong camera consent identity");
          if (
            payload.byteLength !==
            Number(headers["x-buzz-camera-width"]) *
              Number(headers["x-buzz-camera-height"]) *
              4
          )
            throw new Error("Invalid RGBA frame");
          w.__CAMERA_PUSHES__ = (w.__CAMERA_PUSHES__ ?? 0) + 1;
          return true;
        }
        if (command === "read_huddle_camera_video_chunk") {
          await new Promise((resolve) => nativeSetTimeout(resolve, 267));
          if (!share) return null;
          w.__CAMERA_BATCHES__ = (w.__CAMERA_BATCHES__ ?? 0) + 1;
          const packet = packets[index % packets.length];
          const cycle = Math.floor(index / packets.length);
          index++;
          return original("sign_event", {
            kind: 24812,
            tags: [["h", channel]],
            content: JSON.stringify({
              ...packet,
              share,
              sequence: index,
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
    {
      channel: CHANNEL,
      session: SESSION,
      packets: VIDEO,
      throttleWindowTimers,
    },
  );
}

test("camera capture and publication do not depend on throttled window timers", async ({
  page,
}) => {
  await open(page);
  await cameraNative(page, true);
  await page
    .getByRole("button", { name: "Turn camera on", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __CAMERA_PUSHES__?: number })
            .__CAMERA_PUSHES__ ?? 0,
      ),
    )
    .toBeGreaterThan(35);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __CAMERA_BATCHES__?: number })
            .__CAMERA_BATCHES__ ?? 0,
      ),
    )
    .toBeGreaterThan(5);
  await page
    .getByRole("button", { name: "Turn camera off", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __CAMERA_TRACK__?: MediaStreamTrack })
            .__CAMERA_TRACK__?.readyState,
      ),
    )
    .toBe("ended");
});

test("camera stays off until enabled, uses video-only capture and raw IPC, then releases its track", async ({
  page,
}) => {
  await open(page);
  await cameraNative(page);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __CAMERA_REQUESTS__?: unknown[] })
            .__CAMERA_REQUESTS__?.length,
      ),
    )
    .toBe(0);
  await page
    .getByRole("button", { name: "Turn camera on", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("button", { name: "Turn camera off", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __CAMERA_PUSHES__?: number })
            .__CAMERA_PUSHES__ ?? 0,
      ),
    )
    .toBeGreaterThan(15);
  const requests = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __CAMERA_REQUESTS__?: MediaStreamConstraints[];
        }
      ).__CAMERA_REQUESTS__,
  );
  expect(requests?.[0]?.audio).toBe(false);
  await page
    .getByRole("button", { name: "Turn camera off", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { __CAMERA_TRACK__?: MediaStreamTrack })
            .__CAMERA_TRACK__?.readyState,
      ),
    )
    .toBe("ended");
  const pushes = await page.evaluate(
    () =>
      (window as typeof window & { __CAMERA_PUSHES__?: number })
        .__CAMERA_PUSHES__,
  );
  await page.waitForTimeout(500);
  expect(
    await page.evaluate(
      () =>
        (window as typeof window & { __CAMERA_PUSHES__?: number })
          .__CAMERA_PUSHES__,
    ),
  ).toBe(pushes);
});

test("camera video is decoded in participant tiles independently of screen sharing", async ({
  page,
}) => {
  await open(page, false);
  const tile = page.getByTestId("huddle-participant-tile").first();
  await expect(tile).toBeVisible();
  await waitForAnimations(page);
  await tile.screenshot({
    path: "test-results/screenshots/huddle-camera-before.png",
  });
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName: "huddle",
          kind: 24812,
          exactChannel: true,
        }),
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ packets, session, pubkey }) => {
      let index = 0;
      const share = {
        session_id: session,
        started_at: Date.now(),
        label: "Camera",
      };
      (
        window as typeof window & { __CAMERA_TIMER__?: number }
      ).__CAMERA_TIMER__ = window.setInterval(() => {
        const packet = packets[index % packets.length];
        const cycle = Math.floor(index / packets.length);
        const batch = {
          ...packet,
          share,
          sequence: ++index,
          frames: packet.frames.map((frame) => ({
            ...frame,
            timestamp: frame.timestamp + cycle * 6_000_000,
          })),
        };
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "huddle",
          pubkey,
          kind: 24812,
          content: JSON.stringify(batch),
        });
      }, 267);
    },
    { packets: VIDEO, session: SESSION, pubkey: TEST_IDENTITIES.tyler.pubkey },
  );
  const video = page.getByTestId("huddle-camera-video");
  await expect(video).toBeVisible();
  await expect
    .poll(() => video.evaluate((node) => (node as HTMLVideoElement).readyState))
    .toBeGreaterThanOrEqual(2);
  const before = await video.evaluate(
    (node) =>
      (node as HTMLVideoElement).getVideoPlaybackQuality().totalVideoFrames,
  );
  await page.waitForTimeout(1500);
  const after = await video.evaluate(
    (node) =>
      (node as HTMLVideoElement).getVideoPlaybackQuality().totalVideoFrames,
  );
  expect(after - before).toBeGreaterThan(25);
  await expect(page.getByTestId("huddle-shared-screen")).toHaveCount(0);
  await waitForAnimations(page);
  await tile.screenshot({
    path: "test-results/screenshots/huddle-camera-after.png",
  });
  await page.evaluate(
    ({ session, pubkey }) => {
      window.clearInterval(
        (window as typeof window & { __CAMERA_TIMER__?: number })
          .__CAMERA_TIMER__,
      );
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "huddle",
        pubkey,
        kind: 24812,
        content: JSON.stringify({
          type: "stopped",
          share: {
            session_id: session,
            started_at: Date.now() - 1000,
            label: "Camera",
          },
        }),
      });
    },
    { session: SESSION, pubkey: TEST_IDENTITIES.tyler.pubkey },
  );
  await expect(video).toHaveCount(0);
});

test("an absent camera leaves the huddle usable and opening settings refreshes plugged cameras", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const devices = navigator.mediaDevices;
    let present = false;
    devices.enumerateDevices = async () =>
      present
        ? [
            {
              kind: "videoinput",
              deviceId: "usb",
              groupId: "usb",
              label: "USB camera",
              toJSON: () => ({}),
            } as MediaDeviceInfo,
          ]
        : [];
    (
      window as typeof window & { __PLUG_CAMERA__?: () => void }
    ).__PLUG_CAMERA__ = () => {
      present = true;
      devices.dispatchEvent(new Event("devicechange"));
    };
  });
  await installMockBridge(page, {
    windowLabel: `huddle-${CHANNEL}`,
    huddle: {
      parentChannelId: PARENT,
      ephemeralChannelId: CHANNEL,
      members: [],
      transcriptionEnabled: false,
    },
  });
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "No camera detected", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Share screen", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() =>
    (
      window as typeof window & { __PLUG_CAMERA__?: () => void }
    ).__PLUG_CAMERA__?.(),
  );
  await expect(
    page.getByRole("button", { name: "No camera detected", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Camera settings", exact: true })
    .click();
  await expect(
    page.getByRole("menuitemradio", { name: "USB camera", exact: true }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.getByRole("menu", { name: "Camera settings" }).screenshot({
    path: "test-results/screenshots/huddle-camera-settings.png",
  });
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Turn camera on", exact: true }),
  ).toBeEnabled();
});
