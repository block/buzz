import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Per-turn model routing (`crates/buzz-acp/src/routing.rs`) is opt-in through
// the `BUZZ_ROUTING_POLICY` env var, which must name a policy file. This spec
// pins the UI half of that contract: the routing table writes the policy AND
// points the env var at the returned path. Either half alone does nothing —
// a policy file nothing references never gets read, and an env var pointing at
// a missing file makes the harness fail open and route nothing.

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENT_NAME = "Tyler Agent";

async function openAdvanced(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const agentButton = page.getByRole("button", {
    name: `${AGENT_NAME} agent profile`,
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();

  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByRole("button", { name: "Advanced" }).click();
  await expect(page.getByTestId("routing-policy-editor")).toBeVisible({
    timeout: 10_000,
  });
}

test.describe("agent routing policy", () => {
  test("saving a rule writes the policy and points BUZZ_ROUTING_POLICY at it", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openAdvanced(page);

    // Until routing is switched on and saved, the agent carries no policy var.
    await expect(page.getByTestId("routing-policy-save")).toBeVisible();

    await page.getByTestId("routing-policy-enabled").click();
    await page.getByTestId("routing-rule-add").click();

    await page.getByTestId("routing-rule-name").fill("db");
    await page
      .getByTestId("routing-rule-match-kind")
      .selectOption("contains_all");
    await page.getByTestId("routing-rule-phrases").fill("migration, schema");
    await page.getByTestId("routing-rule-model").fill("codex-model");
    await page.getByTestId("routing-default-model").fill("fallback-model");

    const commandsBefore = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_LOG__?.length ?? 0,
    );
    await page.getByTestId("routing-policy-save").click();

    // The policy reaching the backend is the load-bearing half. Assert the
    // snake_case wire shape, not our camelCase view model — the harness parses
    // this document, so a rename here silently disables routing.
    await expect
      .poll(async () =>
        page.evaluate(
          (start) =>
            (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
              .slice(start)
              .find((entry) => entry.command === "set_agent_routing_policy")
              ?.payload ?? null,
          commandsBefore,
        ),
      )
      .toEqual({
        pubkey: AGENT_PUBKEY,
        policy: {
          enabled: true,
          rules: [
            {
              name: "db",
              match_kind: "contains_all",
              any: ["migration", "schema"],
              model: "codex-model",
            },
          ],
          default_model: "fallback-model",
        },
      });

    await expect(page.getByTestId("routing-policy-error")).toHaveCount(0);

    // ...and the other half: the env var now names the saved file. Read the
    // live input values — React controlled inputs do not mirror `value` into a
    // DOM attribute, so an attribute selector would pass vacuously.
    await expect
      .poll(async () => {
        const keys = await page
          .getByTestId("env-vars-key")
          .evaluateAll((nodes) =>
            nodes.map((node) => (node as HTMLInputElement).value),
          );
        const values = await page
          .getByTestId("env-vars-value")
          .evaluateAll((nodes) =>
            nodes.map((node) => (node as HTMLInputElement).value),
          );
        const index = keys.indexOf("BUZZ_ROUTING_POLICY");
        return index === -1 ? null : values[index];
      })
      .toBe(`/mock/agents/routing/${AGENT_PUBKEY}.json`);
  });

  test("a saved policy is read back when the dialog is reopened", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openAdvanced(page);
    await page.getByTestId("routing-policy-enabled").click();
    await page.getByTestId("routing-rule-add").click();
    await page.getByTestId("routing-rule-name").fill("ui");
    await page.getByTestId("routing-rule-phrases").fill("button");
    await page.getByTestId("routing-rule-model").fill("ui-model");
    await page.getByTestId("routing-policy-save").click();
    await expect(page.getByTestId("routing-policy-error")).toHaveCount(0);

    // Close and reopen: the table is hydrated from the stored policy, not from
    // component state that happened to survive.
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();
    await page.getByTestId("user-profile-edit-agent").click();
    await page.getByRole("button", { name: "Advanced" }).click();

    await expect(page.getByTestId("routing-rule-name")).toHaveValue("ui");
    await expect(page.getByTestId("routing-rule-phrases")).toHaveValue(
      "button",
    );
    await expect(page.getByTestId("routing-rule-model")).toHaveValue(
      "ui-model",
    );
  });
});
