import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT_PUBKEY =
  "a0b1c2d3e4f5061728394a5b6c7d8e9f0a1b2c3d4e5f6071829304a5b6c7d8e";

test("owner reviews and can cancel an exact create-only WorkDrive approval", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "Switchboard",
        personaId: "builtin:fizz",
        status: "running",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const content = `Please review this bounded connection upgrade.

\`\`\`buzz:owner-confirmation
${JSON.stringify({
  type: "switchboard_workdrive_profile",
  tenant_id: "tenant-rps",
  channel_id: CHANNEL_ID,
  owner_pubkey: "deadbeef".repeat(8),
  capability_profile: "transcript_upload",
  upgrade_connection_id: "connection-rps",
  scopes: [
    "WorkDrive.files.CREATE",
    "WorkDrive.files.READ",
    "WorkDrive.team.READ",
    "WorkDrive.teamfolders.READ",
    "WorkDrive.users.READ",
    "ZohoFiles.files.READ",
  ],
  actions: ["workdrive.files.upload"],
})}
\`\`\``;
  const request = await page.evaluate(
    ({ content, pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        pubkey,
      }) ?? null,
    { content, pubkey: AGENT_PUBKEY },
  );
  expect(request?.id).toBeTruthy();

  const card = page.getByText("Approve WorkDrive upload access?").locator("..");
  await expect(card).toContainText("Existing files cannot be overwritten");
  await card.getByRole("button", { name: "Review and approve" }).click();
  await expect(
    page.getByRole("heading", { name: "Allow create-only WorkDrive uploads?" }),
  ).toBeVisible();
  await expect(page.getByText("Not allowed:")).toBeVisible();
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(
    page.getByRole("heading", { name: "Allow create-only WorkDrive uploads?" }),
  ).not.toBeVisible();
  await expect(card.getByRole("button", { name: "Review and approve" })).toBeVisible();
});
