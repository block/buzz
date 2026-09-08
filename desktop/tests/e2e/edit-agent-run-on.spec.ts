import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/edit-agent-run-on";

/**
 * The exact persisted shape of a real kubernetes agent created through the
 * app ("Loni" in managed-agents.json, traced by Sami in review): eight keys,
 * `inactivity_seconds` a JSON number, everything else strings, and the
 * optional `service_account` ABSENT — no schema default means the create
 * flow never seeds it, so honest fixtures omit it too.
 */
const KUBERNETES_CONFIG = {
  context: "docker-desktop",
  cpu_limit: "1",
  cpu_request: "1",
  image:
    "ghcr.io/block/buzz-sprig:sha-6530b58@sha256:17facfc7608d8ddb33bc056c9aaba1098f4ef6abe5655702fbfd7584d1f74d76",
  inactivity_seconds: 7200,
  memory_limit: "1Gi",
  memory_request: "1Gi",
  namespace: "buzz-agents-gfq7aq",
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
    "buzz-agents-gfq7aq",
  );
  await expect(runOn.getByTestId("edit-agent-run-on-context")).toContainText(
    "docker-desktop",
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
  // Real records omit the optional service_account (no schema default): the
  // section must show only what was saved, never synthesize a row for it.
  await expect(
    runOn.getByTestId("edit-agent-run-on-service_account"),
  ).toHaveCount(0);

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

test("a blox agent's single saved field renders with a humanized label", async ({
  page,
}) => {
  // The other real provider shape on developer machines: blox records
  // persist exactly one key, exercising provider-generic humanization.
  const agent = TEST_IDENTITIES.bob;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Blox Helper",
        status: "running",
        channelNames: ["general"],
        respondTo: "owner-only",
        backend: {
          type: "provider",
          id: "blox",
          config: { workstation_name: "tlongwell-sprout-home" },
        },
      },
    ],
  });
  await openEditDialog(page, "Blox Helper");

  const runOn = page.getByTestId("edit-agent-run-on");
  await expect(runOn.getByTestId("edit-agent-run-on-location")).toHaveText(
    "blox",
  );
  const row = runOn.getByTestId("edit-agent-run-on-workstation_name");
  await expect(row).toContainText("Workstation name");
  await expect(row).toContainText("tlongwell-sprout-home");

  await runOn.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await page
    .getByTestId("edit-agent-dialog")
    .screenshot({ path: `${SHOTS}/blox-run-on.png` });
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
