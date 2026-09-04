import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, openNewMessagePage } from "../helpers/bridge";

const OWNER = "deadbeef".repeat(8);
const REMOTE = "ed".repeat(32);
const GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const LABEL = "Not managed on this device";

// The delayed fixture exceeds the marker assertion's 5s deadline, proving that
// source readiness is awaited independently of whether a cloud ever renders.
for (const agentListDelayMs of [0, 6_000]) {
  test(`existing owned member uses one cloud across picker, members, author, chip, hover, profile and DM (directory delay ${agentListDelayMs}ms)`, async ({
    page,
  }, testInfo) => {
    await installMockBridge(page, {
      agentListDelayMs,
      managedAgents: [],
      searchProfiles: [
        {
          pubkey: REMOTE,
          displayName: "Remote Scout",
          ownerPubkey: OWNER,
          isAgent: true,
        },
      ],
      relayAgents: [
        {
          pubkey: REMOTE,
          name: "Remote Scout",
          ownerPubkey: OWNER,
          respondTo: "owner-only",
          channelNames: ["general"],
          status: "offline",
        },
      ],
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    // The channel shell can mount before the provider's directory queries settle.
    // Wait for successful source reads, not a delay or a larger marker timeout:
    // the assertions below must still fail if ready data renders no cloud.
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const client = window.__BUZZ_E2E_QUERY_CLIENT__;
            return Object.fromEntries(
              ["identity", "managed-agents", "relay-agents"].map((key) => {
                const state = client?.getQueryState([key]);
                return [
                  key,
                  { status: state?.status, fetchStatus: state?.fetchStatus },
                ];
              }),
            );
          }),
        {
          message:
            "Cloud provenance requires successful identity and agent directory reads",
          timeout: 10_000,
        },
      )
      .toMatchObject({
        identity: { status: "success" },
        "managed-agents": { status: "success" },
        "relay-agents": { status: "success" },
      });
    // Fixture setup only: the UI workflow starts with membership and a DM already present.
    const dm = await page.evaluate(
      async ({ pubkey, channelId }) => {
        await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("add_channel_members", {
          channelId,
          pubkeys: [pubkey],
          role: "bot",
        });
        const dm = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("open_dm", {
          pubkeys: [pubkey],
        })) as { id: string };
        await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
          queryKey: ["channels"],
        });
        return dm;
      },
      { pubkey: REMOTE, channelId: GENERAL },
    );
    await page.getByTestId("channel-general").click();
    const input = page.getByTestId("message-input");
    await input.fill("@Remote");
    const suggestion = page.getByTestId(`mention-suggestion-${REMOTE}`);
    await expect(
      suggestion.getByRole("img", { name: LABEL }).locator("svg.lucide-cloud"),
    ).toBeVisible();
    await waitForAnimations(page);
    await page
      .getByTestId("mention-autocomplete")
      .screenshot({ path: testInfo.outputPath("cloud-picker.png") });
    await input.press("Escape");
    await input.fill("");

    await page.getByTestId("channel-members-trigger").click();
    const members = page.getByTestId("members-sidebar");
    const memberMarker = page.getByTestId(
      `sidebar-member-agent-provenance-${REMOTE}`,
    );
    await expect(memberMarker).toHaveAttribute("aria-label", LABEL);
    await expect(memberMarker.locator("svg.lucide-cloud")).toBeVisible();
    await waitForAnimations(page);
    await members.screenshot({
      path: testInfo.outputPath("cloud-members.png"),
    });
    await members.getByRole("button", { name: "Close", exact: true }).click();

    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }),
        ),
      )
      .toBe(true);
    await page.evaluate((pubkey) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Cloud provenance example: @Remote Scout",
        pubkey,
        // A self-reference is non-notifying, but still resolves a mention chip.
        extraTags: [["mention", pubkey]],
        kind: 40002,
      });
    }, REMOTE);
    const article = page
      .getByTestId("message-row")
      .filter({ hasText: "Cloud provenance example:" });
    await expect(
      article.getByTestId("message-header").getByRole("img", { name: LABEL }),
    ).toBeVisible();
    const chip = article.locator("[data-mention]");
    await expect(chip.locator("svg.lucide-cloud")).toBeVisible();
    await waitForAnimations(page);
    await article.screenshot({
      path: testInfo.outputPath("cloud-author-chip.png"),
    });
    await chip.hover();
    const hover = page.getByTestId("user-profile-popover");
    await expect(
      hover.getByTestId("user-profile-popover-agent-provenance"),
    ).toHaveAttribute("title", LABEL);
    await waitForAnimations(page);
    await hover.screenshot({ path: testInfo.outputPath("cloud-hover.png") });
    await chip.click();
    const panel = page.getByTestId("user-profile-panel");
    await expect(
      panel.getByRole("img", { name: LABEL }).locator("svg.lucide-cloud"),
    ).toBeVisible();
    await waitForAnimations(page);
    await panel.screenshot({ path: testInfo.outputPath("cloud-profile.png") });
    await page.getByTestId("auxiliary-panel-close").click();
    await expect(
      page
        .getByTestId(`channel-agent-provenance-${dm.id}`)
        .locator("svg.lucide-cloud"),
    ).toBeVisible();
    await page.locator(`[data-channel-id="${dm.id}"]`).click();
    await expect(
      page
        .getByTestId("chat-header-agent-provenance")
        .locator("svg.lucide-cloud"),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({ path: testInfo.outputPath("cloud-dm.png") });
    await openNewMessagePage(page);
    await page.getByTestId("new-dm-search").fill("Remote Scout");
    const recipient = page.getByTestId(`new-dm-result-${REMOTE}`);
    await expect(
      recipient.getByRole("img", { name: LABEL }).locator("svg.lucide-cloud"),
    ).toBeVisible();
    await waitForAnimations(page);
    await recipient.screenshot({
      path: testInfo.outputPath("cloud-recipient.png"),
    });
    await page.getByTestId("channel-random").click();
    await page.getByTestId("channel-members-trigger").click();
    await page.getByPlaceholder("Add people and agents").fill("Remote Scout");
    const addResult = page.getByTestId(`channel-user-search-result-${REMOTE}`);
    await expect(
      addResult.getByRole("img", { name: LABEL }).locator("svg.lucide-cloud"),
    ).toBeVisible();
    await waitForAnimations(page);
    await addResult.screenshot({
      path: testInfo.outputPath("cloud-add-member-picker.png"),
    });
    // No invitation or local lifecycle action is exercised by this presentation test.
    await expect(
      page.getByRole("button", { name: "Invite", exact: true }),
    ).toHaveCount(0);
  });
}
