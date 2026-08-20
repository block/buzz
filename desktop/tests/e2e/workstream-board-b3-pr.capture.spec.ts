import { expect, test } from "@playwright/test";

import { installRelayBridge } from "../helpers/bridge";

test("continuous relay-backed Workstream Board bullet 3 PR-status demo", async ({
  page,
}, testInfo) => {
  test.setTimeout(120_000);
  await installRelayBridge(page, "tyler", { seedPreviewFeatures: true });
  await page.routeWebSocket(/localhost:3000/, (socket) => {
    const server = socket.connectToServer();
    let delayedPrSubscription: string | null = null;
    let delayedFrames: string[] = [];
    let releaseScheduled = false;
    const release = () => {
      const frames = delayedFrames;
      delayedFrames = [];
      delayedPrSubscription = null;
      releaseScheduled = false;
      for (const frame of frames) socket.send(frame);
    };
    server.onMessage((message) => {
      if (typeof message === "string") {
        try {
          const payload = JSON.parse(message) as unknown[];
          const type = payload[0];
          const subId = typeof payload[1] === "string" ? payload[1] : null;
          const event = payload[2] as { kind?: unknown } | undefined;
          if (type === "EVENT" && event?.kind === 1618 && subId) {
            delayedPrSubscription = subId;
            delayedFrames.push(message);
            if (!releaseScheduled) {
              releaseScheduled = true;
              setTimeout(release, 2_500);
            }
            return;
          }
          if (type === "EOSE" && subId === delayedPrSubscription) {
            delayedFrames.push(message);
            return;
          }
        } catch {
          // Pass non-JSON relay frames through unchanged.
        }
      }
      socket.send(message);
    });
    socket.onMessage((message) => server.send(message));
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("open-workstream-board-view")).toBeVisible();
  await page.getByTestId("open-workstream-board-view").click();

  const board = page.getByTestId("workstream-board-view");
  await expect(board).toBeVisible();
  const channelId = "eb4d4cee-2b29-4d53-addf-e6c4b93b4c0d";
  // Focused bridge regression: relay mode must return the real kind-40100
  // canvas event. Mock mode intentionally keeps the deterministic null/error
  // behavior covered by relay-connectivity.spec.ts.
  const canvas = (await page.evaluate(async (id) => {
    const tauriWindow = window as Window & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
        command: string,
        payload?: Record<string, unknown>,
      ) => Promise<unknown>;
      __TAURI_INTERNALS__?: {
        invoke?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const invoke =
      tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
      tauriWindow.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Mock invoke bridge is unavailable.");
    return invoke("get_canvas", { channelId: id });
  }, channelId)) as { content?: string | null };
  expect(canvas.content).toContain("B3 linked PR");

  const card = page.getByTestId(`workstream-card-${channelId}`);
  await expect(card).toBeVisible();
  await expect(card).toContainText(
    "Show a real linked PR loading then live status",
  );

  const pullRequest = card.getByTestId("workstream-pull-request");
  await expect(pullRequest).toBeVisible();
  await expect(pullRequest).toContainText("Loading…");
  await page.waitForTimeout(1_000);
  await page.screenshot({
    path: testInfo.outputPath("b3-loading.png"),
    fullPage: true,
  });

  await expect(pullRequest).toContainText("B3 linked PR");
  await expect(pullRequest).toContainText("Open");
  await expect(pullRequest).toContainText("No reviews");
  await page.screenshot({
    path: testInfo.outputPath("b3-live.png"),
    fullPage: true,
  });
  await page.waitForTimeout(3_000);

  await pullRequest.click();
  await expect(page).toHaveURL(/\/projects\//);
  await expect(
    page.locator("[data-project-detail-panel]:visible").first(),
  ).toBeVisible({ timeout: 15_000 });
  await page.screenshot({
    path: testInfo.outputPath("b3-canonical-open.png"),
    fullPage: true,
  });
  await page.waitForTimeout(3_000);
});
