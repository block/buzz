import { expect, test } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const REMOTE = TEST_IDENTITIES.charlie.pubkey;
const LOCAL = "d".repeat(64);
const OWNER = "deadbeef".repeat(8);
const PERSONA = "shared-persona";

for (const hasSibling of [false, true]) {
  test(`explicit relay-only identity has no local controls (${hasSibling ? "local sibling" : "persona only"})`, async ({
    page,
  }, testInfo) => {
    await installMockBridge(page, {
      oaOwnerIsMe: true,
      managedAgents: hasSibling
        ? [
            {
              pubkey: LOCAL,
              name: "Local sibling B",
              personaId: PERSONA,
              status: "running",
              channelNames: ["agents"],
            },
          ]
        : [],
      personas: [
        {
          id: PERSONA,
          displayName: "Shared persona P",
          isActive: true,
          systemPrompt: "Local definition, not the remote identity.",
        },
      ],
      searchProfiles: [
        {
          pubkey: REMOTE,
          displayName: "Relay agent A",
          ownerPubkey: OWNER,
          isAgent: true,
        },
      ],
    });
    await page.goto(`/#/agents?profile=${REMOTE}`);
    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible();
    await expect(page.getByTestId("user-profile-name-row")).toContainText(
      "Relay agent A",
    );
    await expect(
      panel.getByRole("img", { name: "Not managed on this device" }),
    ).toBeVisible();
    for (const testId of [
      "user-profile-agent-primary-action",
      "user-profile-start-agent",
      "user-profile-edit-agent",
      "user-profile-add-to-channel",
    ]) {
      await expect(page.getByTestId(testId)).toHaveCount(0);
    }
    await expect(panel).not.toContainText(
      "Local definition, not the remote identity.",
    );
    await waitForAnimations(page);
    await panel.screenshot({
      path: testInfo.outputPath("exact-relay-identity.png"),
    });

    // Persona-only navigation remains legitimate and intentionally different.
    await page.getByTestId("auxiliary-panel-close").click();
    await page.getByTestId(`persona-agent-row-${PERSONA}`).click();
    await expect(
      page.getByTestId(
        hasSibling
          ? "user-profile-agent-primary-action"
          : "user-profile-start-agent",
      ),
    ).toBeVisible();
    await expect(
      panel.getByRole("img", { name: "Not managed on this device" }),
    ).toHaveCount(0);
  });
}

test("saved deployment with offline presence is not shown as online", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: LOCAL,
        name: "Offline deployment",
        status: "deployed",
        backend: { type: "provider", id: "fixture", config: {} },
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/#/agents");
  const dot = page.getByTestId(`agent-runtime-active-${LOCAL}`);
  await expect(dot).toHaveAttribute(
    "aria-label",
    "Offline deployment: Offline",
  );
  await expect(dot.locator("xpath=../..")).not.toHaveClass(/bg-emerald-500/);
  await page
    .getByRole("button", { name: "Offline deployment agent profile" })
    .click();
  await expect(page.getByTestId("user-profile-presence-badge")).toHaveAttribute(
    "aria-label",
    "Offline",
  );
  // Preserve the existing request-only lifecycle control; no inferred redeploy.
  await expect(
    page.getByTestId("user-profile-agent-primary-action"),
  ).toHaveAttribute("aria-label", "Shutdown");
  await waitForAnimations(page);
  await page
    .getByTestId("user-profile-panel")
    .screenshot({ path: testInfo.outputPath("offline-deployment.png") });

  await page.evaluate(() =>
    window.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?.("disconnected"),
  );
  await expect(page.getByTestId("user-profile-presence-badge")).toHaveCount(0);
  await expect(dot).toHaveAttribute(
    "aria-label",
    "Offline deployment: Availability unknown",
  );
});
