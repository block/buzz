import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const ARTIFACT_EVENT_ID = "7".repeat(64);
const ARTIFACT_URL = `https://mock.relay/media/${"8".repeat(64)}.pdf`;

test("Outbox opens agent artifacts and preserves their source conversation", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("sidebar-primary-menu")).toBeVisible();

  await page.evaluate(
    ({ agentPubkey, artifactEventId, artifactUrl, humanPubkey }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            createdAt: number;
            extraTags: string[][];
            id: string;
            pubkey: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("mock message emitter unavailable");

      emit({
        channelName: "general",
        content: `The launch brief is complete.\n\n[LAUNCH_BRIEF.pdf](${artifactUrl})`,
        createdAt: Math.floor(Date.now() / 1_000),
        extraTags: [
          ["buzz-outbox", "1"],
          [
            "imeta",
            `url ${artifactUrl}`,
            "m application/pdf",
            `x ${"8".repeat(64)}`,
            "size 24576",
            "filename LAUNCH_BRIEF.pdf",
          ],
        ],
        id: artifactEventId,
        pubkey: agentPubkey,
      });

      emit({
        channelName: "general",
        content: `[HUMAN_NOTES.pdf](${artifactUrl})`,
        createdAt: Math.floor(Date.now() / 1_000) + 1,
        extraTags: [
          [
            "imeta",
            `url ${artifactUrl}`,
            "m application/pdf",
            `x ${"9".repeat(64)}`,
            "size 100",
            "filename HUMAN_NOTES.pdf",
          ],
        ],
        id: "9".repeat(64),
        pubkey: humanPubkey,
      });
    },
    {
      agentPubkey: TEST_IDENTITIES.charlie.pubkey,
      artifactEventId: ARTIFACT_EVENT_ID,
      artifactUrl: ARTIFACT_URL,
      humanPubkey: TEST_IDENTITIES.bob.pubkey,
    },
  );

  await page.getByTestId("open-outbox-view").click();
  await expect(page).toHaveURL(/\/outbox$/);
  await expect(page.getByText("LAUNCH_BRIEF.pdf")).toBeVisible();
  await expect(page.getByText("HUMAN_NOTES.pdf")).toHaveCount(0);
  await expect(page.getByTestId("outbox-artifact")).toHaveCount(1);

  await page.getByTestId("open-outbox-artifact").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
          ).__BUZZ_E2E_COMMANDS__?.filter(
            (command) => command === "open_artifact",
          ).length ?? 0,
      ),
    )
    .toBe(1);

  await waitForAnimations(page);
  await page.getByTestId("outbox-screen").screenshot({
    path: "test-results/outbox/outbox.png",
  });

  await page.getByTestId("open-outbox-source").click();
  await expect(page).toHaveURL(
    new RegExp(
      `/channels/${GENERAL_CHANNEL_ID}.*thread=%22${ARTIFACT_EVENT_ID}%22`,
    ),
  );
});
