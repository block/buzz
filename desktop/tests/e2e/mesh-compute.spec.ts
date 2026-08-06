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
    snapshotMemberCount?: number;
    snapshotDevices?: Array<{
      deviceId: string;
      label: string;
      capacityGb: number;
      models: string[];
      state: "serving" | "loading" | "standby" | "consuming";
      isSelf: boolean;
      memberPubkey?: string;
      modelSizeGb?: number;
    }>;
  }) => void;
};

test("Share compute chooses a model before sharing", async ({ page }) => {
  const modelRef = "hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M";
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "compute");
  await page.getByTestId("compute-tab-settings").click();

  const card = page.getByTestId("settings-mesh-share-compute");
  const toggle = page.getByTestId("mesh-share-compute-toggle");
  const model = page.getByTestId("mesh-share-compute-model");

  await expect(card).not.toContainText("Not sharing right now");
  await expect(
    page.getByTestId("mesh-share-compute-options-motion"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("mesh-share-compute-sharing-status"),
  ).toHaveCount(0);
  await expect(model).toBeVisible();
  await expect(toggle).toBeEnabled();
  await model.click();
  await page.getByRole("option", { name: "Custom model…" }).click();
  await page.getByLabel("Custom model reference").fill(modelRef);

  await toggle.click();
  await expect(
    page.getByTestId("mesh-share-compute-options-motion"),
  ).toBeVisible();
  await expect(
    page.getByTestId("mesh-share-compute-sharing-status"),
  ).toBeVisible();
  await expect(model).toBeVisible();
  await expect(card).toContainText(
    "Buzz downloads remote models when sharing starts",
  );
  await expect(toggle).toBeChecked();
  await expect(
    page.getByTestId("mesh-share-compute-sharing-status"),
  ).toContainText("SmolLM2 135M with relay members");
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_start_node");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as E2eWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
      ),
    )
    .toContainEqual({
      command: "mesh_start_node",
      payload: {
        request: { mode: "serve", modelId: modelRef },
      },
    });

  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(card).not.toContainText("Not sharing right now");
  await expect(
    page.getByTestId("mesh-share-compute-options-motion"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("mesh-share-compute-sharing-status"),
  ).toHaveCount(0);
  await expect(model).toBeVisible();
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
  await page.getByTestId("compute-tab-settings").click();

  const card = page.getByTestId("settings-mesh-share-compute");
  const toggle = page.getByTestId("mesh-share-compute-toggle");
  await expect(card).toContainText(
    "This machine is currently using another member's shared compute",
  );
  await expect(card).toContainText("Buzz may briefly restart");
  await expect(toggle).not.toBeChecked();
  await expect(
    page.getByTestId("mesh-share-compute-options-motion"),
  ).toHaveCount(0);
  await expect(toggle).toBeEnabled();
  const customModel = page.getByLabel("Custom model reference");
  await expect(customModel).toHaveValue(localModel);
  await customModel.fill("");
  await expect(customModel).toBeVisible();
  await customModel.fill("hf://demo/replacement-model:Q4_K_M");
  await toggle.click();
  await expect(toggle).toBeChecked();

  const commands = await page.evaluate(() => ({
    names: (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? [],
    payloads: (window as E2eWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
  }));
  expect(commands.names).not.toContain("mesh_stop_node");
  expect(commands.payloads).toContainEqual({
    command: "mesh_start_node",
    payload: {
      request: { mode: "serve", modelId: "hf://demo/replacement-model:Q4_K_M" },
    },
  });
});

test("Shared Compute sidebar row opens the Compute community view", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");

  await page.getByTestId("sidebar-mesh-compute-row").click();

  await expect(page.getByTestId("settings-mesh-compute-page")).toBeVisible();
  await expect(page.getByTestId("compute-tab-community")).toHaveAttribute(
    "data-state",
    "active",
  );
  await expect(page.getByTestId("community-compute-view")).toBeVisible();
  await expect(page.getByTestId("mesh-compute-popover")).toHaveCount(0);
});

test("Community Compute shows live KPIs and the tutorial simulation", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof (window as E2eWindow).__BUZZ_E2E_SET_MESH__ === "function",
  );
  await page.evaluate(() => {
    (window as E2eWindow).__BUZZ_E2E_SET_MESH__?.({
      snapshotMemberCount: 7,
      snapshotDevices: [
        {
          deviceId: "alpha",
          label: "Alpha Studio",
          capacityGb: 64,
          models: ["qwen-32b"],
          state: "serving",
          isSelf: false,
          memberPubkey: "member-alpha",
          modelSizeGb: 28,
        },
        {
          deviceId: "beta",
          label: "Beta Workstation",
          capacityGb: 32,
          models: ["gemma-12b"],
          state: "serving",
          isSelf: false,
          memberPubkey: "member-beta",
          modelSizeGb: 12,
        },
      ],
    });
  });
  await openSettings(page, "compute");

  await expect(page.getByTestId("compute-share-banner")).toBeVisible();
  await expect(
    page.getByTestId("community-compute-kpi-members-sharing"),
  ).toContainText("2");
  await expect(
    page.getByTestId("community-compute-kpi-contributed-vram"),
  ).toContainText("96 GB");
  await expect(
    page.getByTestId("community-compute-kpi-vram-allocated"),
  ).toContainText("42%");
  await expect(
    page.getByTestId("community-compute-territory-map"),
  ).toBeVisible();
  await expect(
    page.getByTestId("community-compute-deployment-list"),
  ).toContainText("Alpha Studio");
  await page.getByLabel("Search deployments").fill("Beta");
  await expect(
    page.getByTestId("community-compute-deployment-list"),
  ).toContainText("Beta Workstation");
  await expect(
    page.getByTestId("community-compute-deployment-list"),
  ).not.toContainText("Alpha Studio");
  await page.getByLabel("Search deployments").fill("");

  const tutorialButton = page.getByTestId("community-compute-tutorial-button");
  await tutorialButton.click();
  await expect(tutorialButton).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByTestId("community-compute-simulated-banner"),
  ).toBeVisible();
  await expect(
    page.getByTestId("community-compute-tutorial-panel"),
  ).toContainText("Every territory is a model deployment");
  await expect(page.getByTestId("compute-share-banner")).toHaveCount(0);
  const tutorialTile = page
    .locator('[data-inference-active="true"]')
    .first()
    .getByTestId("community-compute-tile")
    .first();
  await expect(tutorialTile).toBeVisible();
  await expect
    .poll(() =>
      tutorialTile.evaluate(
        (element) => getComputedStyle(element).animationName,
      ),
    )
    .toBe("mesh-breath");
  await page.getByLabel("Preview scenario").selectOption("single-device");
  await expect(
    page.getByTestId("community-compute-kpi-members-sharing"),
  ).toContainText("1");
  await page.getByRole("button", { name: "Next" }).click();
  await expect(
    page.getByTestId("community-compute-tutorial-panel"),
  ).toContainText("Area represents memory");
  await page.getByRole("button", { name: "Next" }).click();
  await expect(
    page.getByTestId("community-compute-tutorial-panel"),
  ).toContainText("Motion illustrates inference in flight");
  const territory = page.locator("[data-cell-count]").first();
  await territory.click();
  await expect(territory).toHaveAttribute("data-selected", "true");
  await territory.hover();
  await expect(
    page.getByTestId("community-compute-hover-details"),
  ).toContainText("Simulated inference");
  await tutorialButton.click();
  await expect(tutorialButton).toHaveAttribute("aria-pressed", "false");
  await expect(
    page.getByTestId("community-compute-simulated-banner"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("community-compute-kpi-members-sharing"),
  ).toContainText("2");
});
