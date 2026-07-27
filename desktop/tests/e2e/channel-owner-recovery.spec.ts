import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

async function openRecovery(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-management-trigger").click();
  await expect(page.getByTestId("channel-management-sheet")).toBeVisible();
  await page.getByTestId("channel-owner-recovery-open").click();
  await expect(page.getByTestId("channel-owner-recovery-dialog")).toBeVisible();
}

test.describe("channel owner recovery", () => {
  test("eligible community owner reviews policy and confirms promotion", async ({
    page,
  }) => {
    await installMockBridge(page, {
      relayRequiresMembership: true,
      relayRole: "owner",
    });
    await openRecovery(page);

    const dialog = page.getByTestId("channel-owner-recovery-dialog");
    await expect(dialog).toContainText(
      "Every current human owner must have self-archived",
    );
    await expect(dialog).toContainText(
      "A lost or deleted key without that durable consent is not recoverable.",
    );
    await expect(dialog).toContainText(
      "messages, threads, roster, canvas, and workflows remain unchanged",
    );
    await expect(dialog).toContainText("#general");
    await expect(
      page.getByTestId("channel-owner-recovery-current-owners"),
    ).toHaveText("You");
    await expect(dialog).toContainText("bob");
    await waitForAnimations(page);
    await page.screenshot({
      fullPage: true,
      path: "test-results/channel-owner-recovery-dialog.png",
    });

    const submit = page.getByTestId("channel-owner-recovery-submit");
    const reason = page.getByTestId("channel-owner-recovery-reason");
    await expect(submit).toBeDisabled();
    await reason.fill("🙂".repeat(126));
    await expect(
      page.getByText("Audit reason must be at most 500 UTF-8 bytes."),
    ).toBeVisible();
    await expect(submit).toBeDisabled();
    await reason.fill("All current owners recorded prior replacement consent.");
    await expect(dialog).toContainText(
      "All current owners recorded prior replacement consent.",
    );
    await expect(submit).toBeDisabled();
    await page.getByTestId("channel-owner-recovery-confirm").check();
    await expect(submit).toBeEnabled();
    await submit.click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByText("Channel owner recovered")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(
      page.getByText(/was promoted to channel owner by/),
    ).toBeVisible();
    await expect(
      page.getByText(
        /Recovery reason: “All current owners recorded prior replacement consent\.”/,
      ),
    ).toBeVisible();
  });

  test("cancel makes no request and relay denials remain authoritative", async ({
    page,
  }) => {
    const denial =
      "access denied: owner lacks prior durable self-consent naming the target";
    await installMockBridge(page, {
      relayRole: "owner",
      relayRequiresMembership: true,
      channelOwnerRecoveryError: denial,
    });
    await openRecovery(page);
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByTestId("channel-owner-recovery-dialog")).toHaveCount(
      0,
    );

    await page.getByTestId("channel-owner-recovery-open").click();
    await page
      .getByTestId("channel-owner-recovery-reason")
      .fill("Attempt recovery");
    await page.getByTestId("channel-owner-recovery-confirm").check();
    await page.getByTestId("channel-owner-recovery-submit").click();
    await expect(page.getByTestId("channel-owner-recovery-error")).toHaveText(
      denial,
    );
    await expect(
      page.getByTestId("channel-owner-recovery-dialog"),
    ).toBeVisible();
  });

  test("action is absent for non-owners", async ({ page }) => {
    await installMockBridge(page, {
      relayRequiresMembership: true,
      relayRole: "member",
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-management-trigger").click();
    await expect(page.getByTestId("channel-owner-recovery-open")).toHaveCount(
      0,
    );
  });

  test("action is absent for agent identities", async ({ page }) => {
    await installMockBridge(page, {
      relayRole: "owner",
      relayRequiresMembership: true,
      currentIdentityIsAgent: true,
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-management-trigger").click();
    await expect(page.getByTestId("channel-owner-recovery-open")).toHaveCount(
      0,
    );
  });

  test("human community owner need not already belong to the channel", async ({
    page,
  }) => {
    await installMockBridge(page, {
      relayRole: "owner",
      relayRequiresMembership: true,
      currentChannelMemberAbsent: true,
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-management-trigger").click();
    await expect(page.getByTestId("channel-owner-recovery-open")).toBeVisible();
  });
});
