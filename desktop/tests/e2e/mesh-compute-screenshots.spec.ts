import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";
import { waitForAnimations } from "../helpers/animations";

type E2eWindow = Window & {
  __BUZZ_E2E_SET_MESH__?: (mesh: {
    snapshotMemberCount?: number;
    snapshotDevices?: Array<{
      deviceId: string;
      label: string;
      capacityGb: number;
      models: string[];
      state: "serving";
      isSelf: boolean;
      memberPubkey?: string;
      modelSizeGb?: number;
    }>;
  }) => void;
};

test("captures the focused Community Compute topology", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof (window as E2eWindow).__BUZZ_E2E_SET_MESH__ === "function",
  );
  await page.evaluate(() => {
    (window as E2eWindow).__BUZZ_E2E_SET_MESH__?.({
      snapshotMemberCount: 18,
      snapshotDevices: [
        {
          deviceId: "design-studio",
          label: "Design Studio",
          capacityGb: 64,
          models: ["qwen-32b"],
          state: "serving",
          isSelf: true,
          memberPubkey: "design-member",
          modelSizeGb: 28,
        },
        {
          deviceId: "engineering-workstation",
          label: "Engineering Workstation",
          capacityGb: 128,
          models: ["gemma-27b"],
          state: "serving",
          isSelf: false,
          memberPubkey: "engineering-member",
          modelSizeGb: 64,
        },
        {
          deviceId: "research-host",
          label: "Research Host",
          capacityGb: 512,
          models: ["community/research-140b"],
          state: "serving",
          isSelf: false,
          memberPubkey: "research-member",
          modelSizeGb: 350,
        },
      ],
    });
  });
  await openSettings(page, "compute");
  await expect(
    page.getByTestId("community-compute-territory-map"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("settings-panel-compute").screenshot({
    path: "test-results/mesh-compute/community-compute.png",
  });

  await page.locator('[data-deployment-id*="research-host"]').hover();
  await expect(
    page.getByTestId("community-compute-hover-details"),
  ).toContainText("Research Host");
  await waitForAnimations(page);
  await page.getByTestId("community-compute-territory-map").screenshot({
    path: "test-results/mesh-compute/community-map-hover.png",
  });
});
