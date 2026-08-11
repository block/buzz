import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/nxtlinq-authorization";
const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const PROJECT_ROOT = "/Users/demo/project";
const TRUST_STORE = "/Users/operator/nxtlinq/trusted-signers.json";
const RECEIPT_ROOT = "/Users/operator/nxtlinq/receipts";

async function openEditDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: "Nxtlinq Demo agent profile" })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible();
  await page.getByRole("button", { name: "Advanced", exact: true }).click();
  await expect(page.getByTestId("nxtlinq-authorization-preset")).toBeVisible();
}

test.describe("Nxtlinq authorization screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("captures the shared operator settings", async ({ page }) => {
    await installMockBridge(page, {
      nxtlinqAuthorizationConfig: {
        trustStore: TRUST_STORE,
        receiptRoot: RECEIPT_ROOT,
      },
    });
    await page.goto("/");
    await openSettings(page, "agents");

    const card = page.getByTestId("settings-nxtlinq-authorization");
    await expect(card.locator("#nxtlinq-global-trust-store")).toHaveValue(
      TRUST_STORE,
    );
    await expect(card.locator("#nxtlinq-global-receipt-root")).toHaveValue(
      RECEIPT_ROOT,
    );
    await card.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await card.screenshot({ path: `${SHOTS}/01-operator-settings.png` });
  });

  test("captures a verified Agent launch preset", async ({ page }) => {
    await installMockBridge(page, {
      nxtlinqAuthorizationConfig: {
        trustStore: TRUST_STORE,
        receiptRoot: RECEIPT_ROOT,
      },
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: "Nxtlinq Demo",
          runtime: "buzz-agent",
          status: "stopped",
          channelNames: ["agents"],
          workingDirectory: PROJECT_ROOT,
          commandWrapper: {
            command: "/tmp/buzz/bin/nxtlinq-authorization-gateway",
            authorization: {
              kind: "nxtlinq_gateway",
              executable: "/tmp/buzz/bin/nxtlinq-authorization-gateway",
              sha256: "a".repeat(64),
            },
            args: [
              "--adapter",
              "acp",
              "--project",
              PROJECT_ROOT,
              "--trust-store",
              TRUST_STORE,
              "--receipt-dir",
              `${RECEIPT_ROOT}/${AGENT_PUBKEY}`,
              "--mode",
              "acp-enforce",
              "--",
            ],
          },
          envVars: {
            BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE: "1",
            BUZZ_AGENT_REQUIRE_REPLY: "1",
          },
        },
      ],
    });

    await openEditDialog(page);
    const preset = page.getByTestId("nxtlinq-authorization-preset");
    await preset.getByRole("button", { name: "Recheck" }).click();
    await expect(preset.getByRole("button", { name: "Disable" })).toBeVisible();
    await expect(
      preset.getByText("Paths changed", { exact: false }),
    ).toHaveCount(0);
    await waitForAnimations(page);
    await preset.screenshot({ path: `${SHOTS}/02-agent-preset.png` });
  });
});
