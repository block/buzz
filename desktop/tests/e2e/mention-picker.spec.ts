import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const A = "11".repeat(32),
  B = "22".repeat(32);
const GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const OWNER = "deadbeef".repeat(8);
async function seedMembers(page: Page, keys: string[]) {
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.evaluate(
    async ({ keys, channelId }) => {
      await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("add_channel_members", {
        channelId,
        pubkeys: keys,
        role: "bot",
      });
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["relay-agents"],
      });
    },
    { keys, channelId: GENERAL },
  );
}
test.use({ viewport: { width: 1280, height: 900 } });

test("collision distinction, deliberate key choice, and exact publication", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [
      {
        pubkey: A,
        name: "Scout",
        ownerPubkey: OWNER,
        respondTo: "anyone",
        status: "offline",
      },
      {
        pubkey: B,
        name: "Scout",
        ownerPubkey: "aa".repeat(32),
        respondTo: "anyone",
        status: "online",
        channelNames: ["general"],
      },
    ],
    searchProfiles: [A, B].map((pubkey) => ({
      pubkey,
      displayName: "Scout",
      isAgent: true,
    })),
  });
  await page.goto("/");
  await seedMembers(page, [A, B]);
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@Scout");
  await expect(page.getByTestId(`mention-suggestion-${A}`)).toBeVisible();
  const rowIds = await page
    .locator("[data-mention-suggestion-index]")
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-testid")));
  const first = rowIds[1]?.endsWith(A) ? A : B;
  await input.press("ArrowDown");
  // Membership/presence changes affect the next request, not the visible order.
  await page.evaluate(() => {
    const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      setQueryData: (
        key: string[],
        update: (agents: { status: string }[]) => { status: string }[],
      ) => void;
    };
    client.setQueryData(["relay-agents"], (agents) =>
      [...agents].reverse().map((agent) => ({
        ...agent,
        status: agent.status === "online" ? "offline" : "online",
      })),
    );
  });
  await page.waitForTimeout(200);
  await expect
    .poll(() =>
      page
        .locator("[data-mention-suggestion-index]")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-testid")),
        ),
    )
    .toEqual(rowIds);
  await input.press("Tab");
  await expect(input).toHaveText("@Scout ");
  await page.keyboard.type("hello");
  const content = "@Scout hello";
  await expect(input).toHaveText(content);
  await page.getByTestId("send-message").click();
  await expect
    .poll(() =>
      page.evaluate(
        (content) =>
          (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
            .filter((event) => event.content === content)
            .map((event) =>
              event.tags.filter((tag) => tag[0] === "p").map((tag) => tag[1]),
            ),
        content,
      ),
    )
    .toEqual([[first]]);
});

test("Escape discards delayed picker results across navigation", async ({
  page,
}) => {
  await installMockBridge(page, { agentListDelayMs: 1500 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@bo");
  await expect(page.getByTestId("mention-autocomplete-layer")).toContainText(
    "Loading",
  );
  await input.press("Escape");
  await expect(page.getByTestId("mention-autocomplete-layer")).toBeHidden();
  // Await the actual delayed request settling, not an arbitrary quiet period.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_QUERY_CLIENT__?.getQueryState(["relay-agents"])
            ?.status,
      ),
    )
    .toBe("success");
  await expect(page.getByTestId("mention-autocomplete-layer")).toBeHidden();
  await expect(input).toHaveText("@bo");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("mention-autocomplete-layer")).toBeHidden();
  await expect(input).toBeEmpty();
});
