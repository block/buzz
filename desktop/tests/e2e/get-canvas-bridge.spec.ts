import { expect, test } from "@playwright/test";

import { installMockBridge, installRelayBridge } from "../helpers/bridge";

async function invokeCanvas(
  page: import("@playwright/test").Page,
  channelId: string,
) {
  return page.evaluate(async (id) => {
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
  }, channelId);
}

test("relay mode forwards get_canvas to the authenticated kind-40100 query", async ({
  page,
}) => {
  await installRelayBridge(page, "tyler");
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown })
        .__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  const result = (await invokeCanvas(
    page,
    "eb4d4cee-2b29-4d53-addf-e6c4b93b4c0d",
  )) as { content?: string | null; event_id?: string | null };
  expect(result.event_id).toBeTruthy();
  expect(result.content).toContain("B3 linked PR");
});

test("mock mode keeps the deterministic null canvas response", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown })
        .__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  await expect(await invokeCanvas(page, "mock-channel")).toEqual({
    content: null,
    updated_at: null,
    author: null,
  });
});

test("mock mode keeps configured canvas errors isolated", async ({ page }) => {
  await installMockBridge(page, { canvasReadError: "canvas unavailable" });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown })
        .__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  await expect(invokeCanvas(page, "mock-channel")).rejects.toThrow(
    "canvas unavailable",
  );
});
