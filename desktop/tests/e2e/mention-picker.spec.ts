import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, openCreateChannelDialog } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const A = "11".repeat(32),
  B = "22".repeat(32);
const DENIED = "33".repeat(32),
  UNKNOWN = "44".repeat(32),
  INVITE = "55".repeat(32);
const GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const OWNER = "deadbeef".repeat(8);
const parent = process.env.MENTION_PARENT === "1";
async function capture(page: Page, name: string) {
  await waitForAnimations(page);
  await page.screenshot({
    path: `test-results/mention-picker/${parent ? "before" : "after"}-${name}.png`,
    clip: { x: 256, y: 380, width: 1024, height: 520 },
  });
}
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

test("fresh create and Add member then first @ uses governing directory refresh", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [
      {
        pubkey: A,
        name: "Fresh Scout",
        ownerPubkey: OWNER,
        respondTo: "anyone",
      },
    ],
    searchProfiles: [
      {
        pubkey: A,
        displayName: "Fresh Scout",
        isAgent: true,
        ownerPubkey: OWNER,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  // A successful warm-but-old directory is fixture input, not a manual repair.
  await page.evaluate(() => {
    const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      setQueryData: (key: string[], data: unknown) => void;
    };
    client.setQueryData(["relay-agents"], []);
  });
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill("fresh-picker-fixture");
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByTestId("chat-title")).toHaveText(
    "fresh-picker-fixture",
  );
  await page.getByTestId("channel-members-trigger").click();
  await page.getByTestId("channel-management-search-users").fill("Fresh Scout");
  await page.getByTestId(`channel-user-search-result-${A}`).click();
  await expect(page.getByTestId(`sidebar-member-${A}`)).toBeVisible();
  await page
    .getByRole("dialog", { name: "Channel members" })
    .getByRole("button", { name: "Close", exact: true })
    .click();
  await page.getByTestId("message-input").fill("@Fresh");
  const row = page.getByTestId(`mention-suggestion-${A}`);
  await expect(row).toBeVisible();
  await expect(row).toContainText("Member · Mention");
  // Same settled camera on the unchanged parent, even if no row is admitted.
  await page.waitForTimeout(400);
  await capture(page, "fresh-add");
});

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
  expect(rowIds).toEqual([
    `mention-suggestion-${A}`,
    `mention-suggestion-${B}`,
  ]);
  const first = rowIds[1]?.endsWith(A) ? A : B;
  const second = first === A ? B : A;
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
  await input.fill("@Scout");
  await expect
    .poll(() =>
      page
        .locator("[data-mention-suggestion-index]")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-testid")),
        ),
    )
    .toEqual([`mention-suggestion-${first}`, `mention-suggestion-${second}`]);
  await capture(page, "next-open-ranking");
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

test("already visible checking and denied members remain disabled beside permitted Invite", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [
      {
        pubkey: DENIED,
        name: "Verify restricted",
        ownerPubkey: "aa".repeat(32),
        respondTo: "owner-only",
        channelNames: ["general"],
      },
      {
        pubkey: INVITE,
        name: "Verify available",
        ownerPubkey: OWNER,
        respondTo: "anyone",
        status: "away",
      },
    ],
    searchProfiles: [
      { pubkey: DENIED, displayName: "Verify restricted", isAgent: true },
      { pubkey: UNKNOWN, displayName: "Verify pending", isAgent: true },
    ],
  });
  await page.goto("/");
  await seedMembers(page, [DENIED, UNKNOWN]);
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill("@Verify");
  if (!parent)
    await expect(
      page.getByTestId(`mention-suggestion-${INVITE}`),
    ).toBeVisible();
  await page.waitForTimeout(400);
  if (!parent) {
    await expect(
      page
        .getByTestId(`mention-suggestion-${DENIED}`)
        .locator("button")
        .first(),
    ).toBeDisabled();
    await expect(
      page.getByTestId(`mention-suggestion-${UNKNOWN}`),
    ).toContainText("Checking access");
    await expect(
      page.getByTestId(`mention-suggestion-${INVITE}`),
    ).toContainText("Invite…");
  }
  await capture(page, "actions");
  if (!parent) {
    await expect(
      page.getByTestId(`mention-suggestion-${UNKNOWN}`),
    ).toContainText("Unavailable", { timeout: 7000 });
    await expect(
      page.getByRole("button", {
        name: "Retry access check for Verify pending",
      }),
    ).toBeVisible();
    const identities = await page
      .locator("[data-mention-suggestion-index]")
      .evaluateAll((rows) =>
        rows.map((row) => row.getAttribute("data-testid")),
      );
    await page
      .getByRole("button", { name: "Retry access check for Verify pending" })
      .click();
    await expect(
      page.getByTestId(`mention-suggestion-${UNKNOWN}`),
    ).toContainText("Checking access");
    await expect
      .poll(() =>
        page
          .locator("[data-mention-suggestion-index]")
          .evaluateAll((rows) =>
            rows.map((row) => row.getAttribute("data-testid")),
          ),
      )
      .toEqual(identities);
    await expect(page.getByTestId("message-input")).toHaveText("@Verify");
  }
});
