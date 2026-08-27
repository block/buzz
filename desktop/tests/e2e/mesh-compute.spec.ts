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

test("Share compute uses one automatic contribution control", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await openSettings(page, "compute");

  await expect(page.getByTestId("compute-tab-community")).toHaveCount(0);
  await expect(page.getByTestId("compute-tab-settings")).toHaveCount(0);
  await expect(page.getByTestId("settings-mesh-share-compute")).toHaveCount(0);

  const toggle = page.getByTestId("compute-share-banner-toggle");
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(page.getByTestId("compute-share-banner-status")).toContainText(
    "Your tile will appear when it is ready",
  );
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_start_node");

  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect
    .poll(() =>
      page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []),
    )
    .toContain("mesh_stop_node");
});

test("Shared Compute is only available from Settings", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");

  await expect(page.getByTestId("sidebar-mesh-compute-row")).toHaveCount(0);
  await openSettings(page, "compute");

  await expect(page.getByTestId("settings-mesh-compute-page")).toBeVisible();
});

test("Community Compute keeps the live community topology focused", async ({
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
    page.getByTestId("community-compute-territory-map"),
  ).toBeVisible();

  await expect(
    page.getByTestId("community-compute-tutorial-button"),
  ).toHaveCount(0);
  await expect(page.getByTestId("compute-token-leaderboard")).toHaveCount(0);
  await expect(
    page.getByTestId("community-compute-deployment-list"),
  ).toHaveCount(0);
  await expect(page.getByText("Mesh health", { exact: true })).toHaveCount(0);
  await expect(
    page.getByTestId("community-compute-contributor-avatar"),
  ).toHaveCount(0);
  await expect(page.locator('[data-inference-active="true"]')).toHaveCount(0);

  const territory = page.locator("[data-cell-count]").first();
  await territory.hover();
  await expect(
    page.getByTestId("community-compute-hover-details"),
  ).toContainText("Alpha Studio");
  await expect(
    page.getByTestId("community-compute-hover-details"),
  ).toContainText("Available to the community");
});
