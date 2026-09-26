import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const OWNER = "deadbeef".repeat(8);
const REMOTE = "ed".repeat(32);
const OLD = "ec".repeat(32);

for (const uniqueNames of [false, true]) {
  test(`${uniqueNames ? "unique-name hosting" : "client-only"} keeps the offline remote identity stable`, async ({
    page,
  }, testInfo) => {
    // These are the native policy's projected read results. The Rust tests
    // independently exercise the key-generation, deploy and queue-flush gates.
    await installMockBridge(page, {
      agentDevicePolicy: {
        client_only: !uniqueNames,
        unique_names: uniqueNames,
        preferred_agents: [
          {
            relay_url: "https://mock.relay",
            owner_pubkey: OWNER,
            name: "RemoteScout",
            pubkey: REMOTE,
          },
        ],
      },
      managedAgents: [],
      personas: [
        ...(uniqueNames
          ? [
              {
                id: "local-notebook",
                displayName: "Notebook",
                systemPrompt: "Local test agent",
                isActive: true,
              },
            ]
          : []),
        {
          id: "shared-scout",
          displayName: "RemoteScout",
          systemPrompt: "Existing shared definition",
          isActive: false,
        },
      ],
      searchProfiles: [REMOTE, OLD].map((pubkey) => ({
        pubkey,
        displayName: "RemoteScout",
        ownerPubkey: OWNER,
        isAgent: true,
      })),
      relayAgents: [
        {
          pubkey: REMOTE,
          name: "RemoteScout",
          ownerPubkey: OWNER,
          respondTo: "owner-only",
          channelNames: [],
          status: "offline",
        },
      ],
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    for (const suffix of ["first", "again"]) {
      const input = page.getByTestId("message-input");
      await input.fill("@Remote");
      const candidate = page.getByTestId(`mention-suggestion-${REMOTE}`);
      await expect(candidate).toBeVisible();
      await expect(page.getByTestId(`mention-suggestion-${OLD}`)).toHaveCount(
        0,
      );
      await expect(
        page.getByTestId("mention-suggestion-persona-shared-scout"),
      ).toHaveCount(0);
      await candidate.locator("button").first().click();
      await page.keyboard.type(suffix);
      await page.getByTestId("send-message").click();
      if (suffix === "first") {
        await page.getByRole("button", { name: "Invite", exact: true }).click();
      }
      await expect
        .poll(() =>
          page.evaluate(
            (suffix) =>
              (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
                .filter((event) => event.content === `@RemoteScout ${suffix}`)
                .map((event) =>
                  event.tags
                    .filter((tag) => tag[0] === "p")
                    .map((tag) => tag[1]),
                ),
            suffix,
          ),
        )
        .toEqual([[REMOTE]]);
    }
    const commands = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
    );
    for (const command of [
      "create_managed_agent",
      "start_managed_agent",
      "start_managed_agent_runtime",
      "confirm_agent_snapshot_import",
    ]) {
      expect(commands.some((call) => call.command === command)).toBe(false);
    }
    const adds = commands.filter(
      (call) => call.command === "add_channel_members",
    );
    expect(adds).toHaveLength(1);
    expect(adds[0].payload).toMatchObject({ pubkeys: [REMOTE], role: "bot" });
    if (uniqueNames) {
      const input = page.getByTestId("message-input");
      await input.fill("@Notebook");
      await expect(
        page.getByTestId("mention-suggestion-persona-local-notebook"),
      ).toBeVisible();
      await input.press("Enter");
      await page.keyboard.type(" local hello");
      await page.getByTestId("send-message").click();
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
                (c) => c.command === "create_managed_agent",
              ).length,
          ),
        )
        .toBe(1);
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
                (c) => c.command === "start_managed_agent",
              ).length,
          ),
        )
        .toBe(1);
      const created = await page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).find(
          (c) => c.command === "create_managed_agent",
        ),
      );
      expect(created?.payload).toMatchObject({ input: { name: "Notebook" } });
      await expect
        .poll(() =>
          page.evaluate(() =>
            (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
              .filter((e) => e.content.includes("local hello"))
              .flatMap((e) =>
                e.tags.filter((t) => t[0] === "p").map((t) => t[1]),
              ),
          ),
        )
        .not.toEqual([]);
    }
    await waitForAnimations(page);
    await page.screenshot({
      path: testInfo.outputPath("client-only-stable-recipient.png"),
    });
  });
}
