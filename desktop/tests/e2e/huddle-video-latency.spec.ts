import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import type { VideoBatch } from "../../src/features/huddle/lib/screenShare";

const CHANNEL = "11111111-1111-4111-8111-111111111111";
const VIDEO = JSON.parse(
  readFileSync(
    new URL("../fixtures/huddle-video.json", import.meta.url),
    "utf8",
  ),
) as VideoBatch[];

for (const [kind, testId, label] of [
  [24811, "huddle-shared-screen", "screen"],
  [24812, "huddle-camera-video", "camera"],
] as const) {
  test(`${label} presents fresh decoded frames instead of replaying a cached second`, async ({
    page,
  }) => {
    await installMockBridge(page, {
      windowLabel: `huddle-${CHANNEL}`,
      huddle: {
        parentChannelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        ephemeralChannelId: CHANNEL,
        members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
        transcriptionEnabled: false,
      },
    });
    await page.goto("/");
    await expect
      .poll(() =>
        page.evaluate(
          (kind) =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "huddle",
              kind,
              exactChannel: true,
            }),
          kind,
        ),
      )
      .toBe(true);

    // Observe actual compositor presentations before the player is mounted.
    await page.evaluate(
      ({ packets, kind, testId, pubkey }) => {
        const w = window as typeof window & {
          __LATENCY_PRESENTATIONS__?: number[];
          __LATENCY_OBSERVER__?: MutationObserver;
        };
        const presented: number[] = [];
        const tracked = new WeakSet<HTMLVideoElement>();
        w.__LATENCY_PRESENTATIONS__ = presented;
        const track = () => {
          for (const video of document.querySelectorAll<HTMLVideoElement>(
            `[data-testid="${testId}"]`,
          )) {
            if (tracked.has(video)) continue;
            tracked.add(video);
            const next = (
              _now: number,
              metadata: VideoFrameCallbackMetadata,
            ) => {
              presented.push(metadata.mediaTime);
              if (presented.length < 3) video.requestVideoFrameCallback(next);
            };
            video.requestVideoFrameCallback(next);
          }
        };
        const observer = new MutationObserver(track);
        observer.observe(document.body, { childList: true, subtree: true });
        w.__LATENCY_OBSERVER__ = observer;
        const share = {
          session_id: "22222222-2222-4222-8222-222222222222",
          started_at: Date.now(),
          label: "Live latency regression",
        };
        // Real OpenH264 frames: a warm dependency chain ending at 767 ms.
        for (const [index, packet] of packets.entries()) {
          window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName: "huddle",
            kind,
            pubkey,
            content: JSON.stringify({ ...packet, share, sequence: index + 1 }),
          });
        }
      },
      {
        packets: VIDEO.slice(0, 3),
        kind,
        testId,
        pubkey: TEST_IDENTITIES.tyler.pubkey,
      },
    );
    const video = page.getByTestId(testId).first();
    await expect(video).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as typeof window & {
                __LATENCY_PRESENTATIONS__?: number[];
              }
            ).__LATENCY_PRESENTATIONS__?.length ?? 0,
        ),
      )
      .toBeGreaterThanOrEqual(3);
    const presentations = await page.evaluate(
      () =>
        (
          window as typeof window & {
            __LATENCY_PRESENTATIONS__?: number[];
          }
        ).__LATENCY_PRESENTATIONS__ ?? [],
    );
    expect(Math.max(...presentations.slice(0, 3))).toBeGreaterThanOrEqual(0.4);
    expect(0.8 - Math.max(...presentations.slice(0, 3))).toBeLessThan(0.45);
    await expect(page.getByRole("alert")).toHaveCount(0);
    await page.evaluate(() => {
      (
        window as typeof window & {
          __LATENCY_OBSERVER__?: MutationObserver;
        }
      ).__LATENCY_OBSERVER__?.disconnect();
    });
  });
}
