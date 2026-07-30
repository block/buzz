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
    communityRelayUrl?: string | null;
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
  await expect(page.getByTestId("mesh-community-binding")).toContainText(
    "When enabled, the selected model will be tied to E2E Test",
  );
  await expect(page.getByTestId("mesh-community-binding")).toContainText(
    "stop sharing before enabling it there",
  );
  await expect(model).not.toBeVisible();
  await expect(toggle).toBeDisabled();
  await page.getByTestId("mesh-smart-recommend").click();
  const scan = page.getByTestId("mesh-hardware-scan");
  await expect(scan).toContainText("Chip");
  await expect(scan).toContainText("AI memory");
  await expect(scan).toContainText("Free disk");
  await expect(scan).toContainText("Best model");
  await expect(page.getByTestId("mesh-recommendation-result")).toBeVisible({
    timeout: 4_000,
  });
  await expect(model).not.toBeVisible();
  await page.getByTestId("mesh-use-recommendation").click();
  await expect(
    page.getByTestId("mesh-smart-recommendation-selected"),
  ).toContainText("Qwen3-Coder-Next-Q4_K_M");
  await expect(model).toHaveValue("Qwen3-Coder-Next-Q4_K_M");
  await expect(toggle).toBeEnabled();

  // A member can still override the explicit recommendation.
  await model.fill("hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M");
  await expect(card).toContainText(
    "Buzz downloads remote models when sharing starts",
  );
  await expect(toggle).toBeEnabled();

  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(card).toContainText(
    "Sharing SmolLM2 135M with E2E Test members",
  );
  await expect(page.getByTestId("mesh-community-binding")).toContainText(
    "SmolLM2 135M is tied to E2E Test",
  );
  await expect(page.getByTestId("mesh-community-binding")).toContainText(
    "Switching communities will not move or stop it",
  );
  await expect(card).toContainText(
    "Only members of E2E Test can discover and use this shared model",
  );
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

test("sharing stays pinned to the community where it was enabled", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-communities",
      JSON.stringify([
        {
          id: "e2e-default-community",
          name: "Current Community",
          relayUrl: "ws://localhost:3000",
          addedAt: new Date().toISOString(),
        },
        {
          id: "shared-community",
          name: "Compute Commons",
          relayUrl: "wss://compute.example",
          addedAt: new Date().toISOString(),
        },
      ]),
    );
    window.localStorage.setItem(
      "buzz.mesh-compute.share.model.v1",
      "hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M",
    );
  });
  await page.goto("/");
  await page.waitForFunction(
    () => typeof (window as E2eWindow).__BUZZ_E2E_SET_MESH__ === "function",
  );
  await page.evaluate(() => {
    (window as E2eWindow).__BUZZ_E2E_SET_MESH__?.({
      nodeState: "running",
      nodeMode: "serve",
      communityRelayUrl: "wss://compute.example",
    });
  });
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  await expect(page.getByTestId("mesh-share-compute-toggle")).toBeChecked();
  await expect(page.getByTestId("mesh-sharing-community")).toContainText(
    "Sharing SmolLM2 135M with Compute Commons",
  );
  await expect(card).toContainText(
    "Switch to that community to manage its compute settings",
  );
  const binding = page.getByTestId("mesh-community-binding");
  await expect(binding).toContainText(
    "SmolLM2 135M is tied to Compute Commons",
  );
  await expect(binding).toContainText(
    "Switching communities will not move or stop it",
  );
  await expect(card).toContainText(
    "Only members of Compute Commons can discover and use this shared model",
  );
  await expect(card).toContainText("Other Buzz communities cannot access it");
  const commandsBeforeStop = await page.evaluate(
    () => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? [],
  );
  expect(commandsBeforeStop).not.toContain("mesh_start_node");
  expect(commandsBeforeStop).not.toContain("mesh_stop_node");

  await page.getByTestId("mesh-share-compute-toggle").click();
  await expect(page.getByTestId("mesh-share-compute-toggle")).not.toBeChecked();
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_stop_node");
});

test("accepted smart recommendation is restored when Settings is reopened", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "compute");

  await expect(page.getByTestId("mesh-smart-recommend")).toBeVisible();
  await page.getByTestId("mesh-smart-recommend").click();
  await expect(page.getByTestId("mesh-recommendation-result")).toBeVisible({
    timeout: 4_000,
  });
  await page.getByTestId("mesh-use-recommendation").click();

  // Settings is route-backed, so reload restores the current Compute section;
  // there is no app-shell settings button to click while already inside it.
  await page.reload();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(
    page.getByTestId("mesh-smart-recommendation-selected"),
  ).toContainText("Qwen3-Coder-Next-Q4_K_M");
  await expect(page.getByTestId("mesh-smart-recommend")).not.toBeVisible();
  await expect(page.getByTestId("mesh-share-compute-model")).toHaveValue(
    "Qwen3-Coder-Next-Q4_K_M",
  );
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
