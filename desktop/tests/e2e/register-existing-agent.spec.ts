import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const SOURCE_AGENT = TEST_IDENTITIES.charlie.pubkey;
const REMOTE_AGENT = TEST_IDENTITIES.bob.pubkey;

test("reviews and registers an owner-attested remote identity without creating a local runtime", async ({
  page,
}) => {
  await installMockBridge(page, {
    archivedIdentities: [],
    managedAgents: [
      {
        pubkey: SOURCE_AGENT,
        name: "Registry helper",
        status: "running",
        channelNames: ["agents"],
      },
    ],
    oaOwnerIsMe: true,
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
  );

  await page.evaluate(
    ({ agentPubkey, channelId, remoteAgent }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq: 1,
            timestamp: new Date().toISOString(),
            kind: "tool_result",
            agentIndex: 0,
            channelId,
            sessionId: "registry-session",
            turnId: "registry-turn",
            payload: {
              type: "agent_management_request",
              action: "adopt",
              requestId: "register-remote-agent",
              request: {
                channelId,
                agentPubkey: remoteAgent,
                displayName: "Luci",
              },
            },
          },
        ],
      });
    },
    {
      agentPubkey: SOURCE_AGENT,
      channelId: CHANNEL_ID,
      remoteAgent: REMOTE_AGENT,
    },
  );

  const review = page.getByTestId("register-existing-agent-review");
  await expect(review).toBeVisible();
  await expect(review).toContainText("Register existing agent");
  await expect(review).toContainText("Luci");
  await expect(review).toContainText(REMOTE_AGENT);
  await expect(review).toContainText("Owner only");
  await expect(
    review.getByRole("button", { name: "Register identity" }),
  ).toBeEnabled();
  await review.screenshot({
    path: "test-results/register-existing-agent/01-owner-review.png",
  });
  await review.getByRole("button", { name: "Register identity" }).click();
  await expect(review).not.toBeVisible();

  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
  );
  const registration = commands.find(
    (entry) => entry.command === "register_existing_relay_agent",
  );
  expect(registration?.payload).toMatchObject({
    input: {
      agentPubkey: REMOTE_AGENT,
      displayName: "Luci",
      expectedOwnerPubkey: expect.any(String),
      expectedRelayUrl: expect.any(String),
      expectedSignerPubkey: expect.any(String),
    },
  });
  expect(commands.map((entry) => entry.command)).not.toContain(
    "create_managed_agent",
  );
});
