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
    await waitForAnimations(page);
    await panel.screenshot({
      path: testInfo.outputPath("explicit-persona.png"),
    });
  });
}

for (const allArchived of [false, true]) {
  test(`archived exact key stays navigable (${allArchived ? "all archived" : "live sibling"})`, async ({
    page,
  }) => {
    await installMockBridge(page, {
      oaOwnerIsMe: true,
      archivedIdentities: allArchived ? [REMOTE, LOCAL] : [REMOTE],
      managedAgents: [
        {
          pubkey: REMOTE,
          name: "Archived A",
          personaId: PERSONA,
          status: "stopped",
          channelNames: ["agents"],
        },
        {
          pubkey: LOCAL,
          name: "Sibling B",
          personaId: PERSONA,
          status: "running",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: PERSONA,
          displayName: "Shared persona P",
          isActive: true,
          systemPrompt: "Archive profile fixture.",
        },
      ],
    });
    await page.goto(`/#/agents?profile=${REMOTE}`);
    await expect(page.getByTestId("user-profile-panel")).toBeVisible();
    await expect(page.getByTestId("user-profile-archived-flair")).toBeVisible();
    await expect(
      page.getByTestId("user-profile-agent-primary-action"),
    ).toHaveAttribute("aria-label", "Start agent");
    await expect(page.getByTestId("user-profile-name-row")).toContainText(
      "Archived A",
    );
    // Persona navigation still excludes archived representatives.
    await page.getByTestId("auxiliary-panel-close").click();
    await page.getByTestId(`persona-agent-row-${PERSONA}`).click();
    if (allArchived) {
      await expect(page.getByTestId("user-profile-start-agent")).toBeVisible();
      await expect(
        page.getByTestId("user-profile-agent-primary-action"),
      ).toHaveCount(0);
    } else {
      await expect(
        page.getByTestId("user-profile-agent-primary-action"),
      ).toHaveAttribute("aria-label", "Stop");
    }
  });
}
