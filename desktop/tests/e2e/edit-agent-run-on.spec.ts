import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/edit-agent-run-on";

/**
 * The saved kubernetes config a real create flow persists: every field the
 * provider's `info` schema prefills (see
 * `crates/buzz-backend-kubernetes/src/config.rs::config_schema`), with the
 * digest-pinned sprig image and a generated namespace.
 */
const KUBERNETES_CONFIG = {
  context: "prod-us-west",
  namespace: "buzz-agents-x7k2mp",
  image:
    "ghcr.io/block/buzz-sprig:sha-6530b58@sha256:17facfc7e2cf5d4b21bb8dbdbeb6b0a4c62be2fc1dd1523cd0dcf3e977a2fb63",
  cpu_request: "1",
  memory_request: "2Gi",
  cpu_limit: "2",
  memory_limit: "4Gi",
  inactivity_seconds: 7200,
  service_account: "buzz-agents",
};

async function openEditDialog(
  page: import("@playwright/test").Page,
  agentName: string,
) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: `${agentName} agent profile` })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible();
}

test("editing a kubernetes agent shows its saved run-on settings", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Remote Helper",
        status: "running",
        channelNames: ["general"],
        respondTo: "owner-only",
        backend: {
          type: "provider",
          id: "kubernetes",
          config: KUBERNETES_CONFIG,
        },
      },
    ],
  });
  await openEditDialog(page, "Remote Helper");

  const runOn = page.getByTestId("edit-agent-run-on");
  await expect(runOn).toBeVisible();
  await expect(runOn.getByTestId("edit-agent-run-on-location")).toHaveText(
    "kubernetes",
  );

  // Every saved field renders as a labeled row with its stored value.
  await expect(runOn.getByTestId("edit-agent-run-on-namespace")).toContainText(
    "buzz-agents-x7k2mp",
  );
  await expect(runOn.getByTestId("edit-agent-run-on-context")).toContainText(
    "prod-us-west",
  );
  await expect(runOn.getByTestId("edit-agent-run-on-image")).toContainText(
    "ghcr.io/block/buzz-sprig",
  );
  await expect(
    runOn.getByTestId("edit-agent-run-on-cpu_request"),
  ).toContainText("CPU request");
  await expect(
    runOn.getByTestId("edit-agent-run-on-inactivity_seconds"),
  ).toContainText("7200");
  await expect(
    runOn.getByTestId("edit-agent-run-on-service_account"),
  ).toContainText("buzz-agents");

  // The section explains immutability instead of pretending to be a form.
  await expect(runOn).toContainText("can't be changed afterwards");

  await runOn.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await page
    .getByTestId("edit-agent-dialog")
    .screenshot({ path: `${SHOTS}/kubernetes-run-on.png` });
});

test("editing a local agent names this computer, with no config rows", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.tyler;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Local Helper",
        status: "stopped",
        channelNames: ["general"],
        respondTo: "owner-only",
        backend: { type: "local" },
      },
    ],
  });
  await openEditDialog(page, "Local Helper");

  const runOn = page.getByTestId("edit-agent-run-on");
  await expect(runOn.getByTestId("edit-agent-run-on-location")).toHaveText(
    "This computer",
  );
  await expect(runOn.getByTestId("edit-agent-run-on-namespace")).toHaveCount(0);

  await runOn.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await page
    .getByTestId("edit-agent-dialog")
    .screenshot({ path: `${SHOTS}/local-run-on.png` });
});

test("secret-shaped keys from an untrusted provider render redacted", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Future Provider Agent",
        status: "running",
        channelNames: ["general"],
        respondTo: "owner-only",
        backend: {
          type: "provider",
          id: "some-future-provider",
          config: {
            endpoint: "https://provider.example",
            api_token: "tok-do-not-show",
          },
        },
      },
    ],
  });
  await openEditDialog(page, "Future Provider Agent");

  const runOn = page.getByTestId("edit-agent-run-on");
  await expect(runOn.getByTestId("edit-agent-run-on-endpoint")).toContainText(
    "https://provider.example",
  );
  const tokenRow = runOn.getByTestId("edit-agent-run-on-api_token");
  await expect(tokenRow).toContainText("••••••••");
  await expect(tokenRow).not.toContainText("tok-do-not-show");
  // The raw secret must not appear anywhere in the dialog.
  await expect(page.getByTestId("edit-agent-dialog")).not.toContainText(
    "tok-do-not-show",
  );

  await runOn.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await page
    .getByTestId("edit-agent-dialog")
    .screenshot({ path: `${SHOTS}/redacted-run-on.png` });
});
