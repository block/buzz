import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Routing pin for rule 19 ("a provider record answers from itself"): every
// provider-created agent carries a personaId, so the profile panel's Edit
// action used to hand the whole remote family to the DEFINITION dialog. That
// dialog reads an AgentDefinition, whose projection has no slot for
// backend/agent_command — so a remote agent opened on a blank harness, the
// local default was re-seeded over it, and Save was blocked behind a provider
// + API-key demand for a machine the agent never runs on.
//
// The instance dialog is the remote-aware surface (it renders the record's own
// pin instead of this computer's catalog), so a provider-backed record edits
// there.

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const PERSONA_ID = "persona-provider-routing-e2e";

test.describe("provider-backed edit routing", () => {
  test("opens the instance editor, not the definition editor", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Remote Goose",
          personaId: PERSONA_ID,
          status: "deployed",
          channelNames: ["agents"],
          backend: { type: "provider", id: "ssh", config: {} },
        },
      ],
      personas: [
        {
          id: PERSONA_ID,
          displayName: "Remote Goose",
          systemPrompt: "You are the provider-routing e2e persona.",
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();

    // Persona-linked agents render grouped under the persona's card name.
    const agentButton = page.getByRole("button", {
      name: "Remote Goose agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    // The instance editor opens, showing the record's OWN harness pin …
    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId("edit-agent-pinned-harness")).toBeVisible({
      timeout: 10_000,
    });
    // … and the definition dialog — whose harness dropdown would be blank and
    // then re-seeded with this computer's default — never mounts.
    await expect(page.getByTestId("persona-dialog")).not.toBeVisible();
    await expect(page.locator("#persona-runtime")).toHaveCount(0);
  });
});
