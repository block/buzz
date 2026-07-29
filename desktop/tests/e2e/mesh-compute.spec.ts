import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

type E2eWindow = Window & {
  __BUZZ_E2E_COMMANDS__?: string[];
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: { request?: { mode?: string; modelId?: string } } | null;
  }>;
  __BUZZ_E2E_SET_MESH__?: (mesh: {
    nodeState?: "off" | "running";
    nodeMode?: "serve" | "client" | null;
  }) => void;
};

test("Share compute has a clear empty state and starts and stops sharing", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  const toggle = page.getByTestId("mesh-share-compute-toggle");
  const model = page.getByTestId("mesh-share-compute-model");

  await expect(card).toContainText("Not sharing right now");
  await expect(card).toContainText(
    "Choose a suggested model below, or enter a model reference or local file",
  );
  await expect(toggle).toBeDisabled();

  await model.fill("hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M");
  await expect(card).toContainText(
    "Buzz downloads remote models when sharing starts",
  );
  await expect(toggle).toBeEnabled();

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(card).toContainText("Sharing SmolLM2 135M with relay members");
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_start_node");

  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(card).toContainText("Not sharing right now");
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_stop_node");
});

test("a consuming client can switch to sharing its saved local model", async ({
  page,
}) => {
  // Regression: consuming someone else's shared compute starts a client-mode
  // node in the single runtime slot, which reports state:"running". The Share
  // toggle keyed off state alone and lit up. A later guard overcorrected by
  // disabling the switch and copying the remote model over the local sharing
  // choice. Keep the switch off, preserve the local model, then replace the
  // client with one serve start (never a stop command).
  const localModel = "hf://demo/local-small-model:Q4_K_M";
  await page.addInitScript((model) => {
    window.localStorage.setItem("buzz.mesh-compute.share.model.v1", model);
  }, localModel);
  await installMockBridge(page);
  await page.goto("/");
  // The mesh seed hook is installed when the mock bridge boots; calling it
  // before then silently no-ops (optional chaining) and the seed is lost.
  await page.waitForFunction(
    () => typeof (window as E2eWindow).__BUZZ_E2E_SET_MESH__ === "function",
  );
  await page.evaluate(() => {
    (window as E2eWindow).__BUZZ_E2E_SET_MESH__?.({
      nodeState: "running",
      nodeMode: "client",
    });
  });
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  const toggle = page.getByTestId("mesh-share-compute-toggle");
  const model = page.getByTestId("mesh-share-compute-model");

  await expect(card).toContainText(
    "This machine is currently using another member's shared compute",
  );
  await expect(card).toContainText("Buzz may briefly restart");
  await expect(toggle).not.toBeChecked();
  await expect(model).toBeEnabled();
  await expect(model).toHaveValue(localModel);
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).toBeChecked();

  const commands = await page.evaluate(() => ({
    names: (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? [],
    payloads: (window as E2eWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
  }));
  expect(commands.names).not.toContain("mesh_stop_node");
  expect(commands.payloads).toContainEqual({
    command: "mesh_start_node",
    payload: { request: { mode: "serve", modelId: localModel } },
  });
});
