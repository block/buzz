import { waitForAnimations } from "../helpers/animations";
import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const OWNER = "deadbeef".repeat(8);
const REMOTE = "ed".repeat(32);

async function install(page: Page) {
  await installMockBridge(page, {
    ownerOnlyAccessBuild: true,
    managedAgents: [],
    searchProfiles: [
      {
        pubkey: REMOTE,
        displayName: "RemoteScout",
        ownerPubkey: OWNER,
        isAgent: true,
      },
    ],
    relayAgents: [
      {
        pubkey: REMOTE,
        name: "RemoteScout",
        ownerPubkey: OWNER,
        respondTo: "allowlist",
        respondToAllowlist: [],
        channelNames: [],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}
async function select(page: Page) {
  await page.getByTestId("message-input").fill("@Remote");
  const row = page.getByTestId(`mention-suggestion-${REMOTE}`);
  await expect(row).toContainText("RemoteScout");
  await row.locator("button").first().click();
  await page.keyboard.type("hello");
}
async function sent(page: Page) {
  return page.evaluate(() => {
    const signed = (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
      .filter((event) => event.content === "@RemoteScout hello")
      .map((event) =>
        event.tags.filter((tag) => tag[0] === "p").map((tag) => tag[1]),
      );
    if (signed.length) return signed;
    // New DMs deliberately use the acknowledged native HTTP command rather
    // than JS sign_event. Assert its exact outgoing recipients, not fake crypto.
    return (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).flatMap((call) => {
      const payload = call.payload as {
        content?: string;
        mentionPubkeys?: string[];
      };
      return call.command === "send_channel_message" &&
        payload.content === "@RemoteScout hello"
        ? [payload.mentionPubkeys ?? []]
        : [];
    });
  });
}
async function assertNoLocalLifecycle(page: Page) {
  const commands = await page.evaluate(
    () => window.__BUZZ_E2E_COMMANDS__ ?? [],
  );
  for (const command of [
    "start_managed_agent",
    "create_managed_agent",
    "attach_managed_agent",
  ]) {
    expect(commands).not.toContain(command);
  }
}
const FORUM = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";

async function openStandaloneForumInvite(page: Page) {
  await install(page);
  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await page.getByRole("button", { name: "Start a new post..." }).click();
  await select(page);
  await page.getByTestId("send-message").click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("RemoteScout");
  await expect(
    dialog.getByRole("button", { name: "Invite", exact: true }),
  ).toBeVisible();
  expect(await sent(page)).toEqual([]);
  return dialog;
}

async function forumAdds(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
      (call) => call.command === "add_channel_members",
    ),
  );
}

test("standalone forum explicitly invites, refreshes membership, then sends exact p-tags", async ({
  page,
}) => {
  const dialog = await openStandaloneForumInvite(page);
  expect(await forumAdds(page)).toEqual([]);
  await waitForAnimations(page);
  await page.screenshot({ path: "test-results/forum-invite.png" });
  // Unlike chat's explicit reference-only option, cancel must not publish.
  await expect(
    dialog.getByRole("button", { name: /Do nothing|Send anyway/ }),
  ).toHaveCount(0);
  await dialog.getByRole("button", { name: "Invite", exact: true }).click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
  await waitForAnimations(page);
  await page.screenshot({ path: "test-results/forum-sent.png" });
  expect(await forumAdds(page)).toEqual([
    expect.objectContaining({
      payload: expect.objectContaining({
        channelId: FORUM,
        pubkeys: [REMOTE],
        role: "bot",
      }),
    }),
  ]);
  const calls = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
  );
  const addIndex = calls.findIndex(
    (call) => call.command === "add_channel_members",
  );
  expect(calls.slice(0, addIndex)).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "revalidate_relay_agents",
        payload: expect.objectContaining({
          channelId: FORUM,
          pubkeys: [REMOTE],
        }),
      }),
    ]),
  );
  const afterAdd = calls.slice(addIndex + 1);
  const refreshIndex = afterAdd.findIndex(
    (call) => call.command === "get_channel_members",
  );
  const finalCheckIndex = afterAdd.findIndex(
    (call) => call.command === "revalidate_relay_agents",
  );
  expect(refreshIndex).toBeGreaterThanOrEqual(0);
  expect(finalCheckIndex).toBeGreaterThan(refreshIndex);
  await assertNoLocalLifecycle(page);
});

test("standalone forum cancelled Invite keeps selected identity and draft without publishing", async ({
  page,
}) => {
  const dialog = await openStandaloneForumInvite(page);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  expect(await sent(page)).toEqual([]);
  expect(await forumAdds(page)).toEqual([]);
  // Retry without selecting again must preserve the exact intended recipient.
  await page.getByTestId("send-message").click();
  await page
    .getByRole("alertdialog")
    .getByRole("button", { name: "Invite", exact: true })
    .click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
});

for (const error of [
  "actor not authorized",
  "policy:nobody — this agent has disabled external channel additions",
  "relay unavailable during add",
]) {
  test(`standalone forum denied/failed add preserves draft and publishes nothing: ${error}`, async ({
    page,
  }) => {
    const dialog = await openStandaloneForumInvite(page);
    await page.evaluate((error) => {
      window.__BUZZ_E2E__.mock ??= {};
      window.__BUZZ_E2E__.mock.addChannelMembersErrors = [error];
    }, error);
    await dialog.getByRole("button", { name: "Invite", exact: true }).click();
    await expect(dialog.getByText(error, { exact: true })).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/forum-error-${error.split(" ")[0]}.png`,
    });
    await expect(page.getByTestId("message-input")).toHaveText(
      "@RemoteScout hello",
    );
    expect(await sent(page)).toEqual([]);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(page.getByTestId("message-input")).toHaveAttribute(
      "contenteditable",
      "true",
    );
    expect(await sent(page)).toEqual([]);
    await assertNoLocalLifecycle(page);
  });
}

test("standalone forum selected identity revoked before Invite fails visibly without adding", async ({
  page,
}) => {
  const dialog = await openStandaloneForumInvite(page);
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentRevalidationRevokedPubkeys = [pubkey];
  }, REMOTE);
  await dialog.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(
    dialog.getByText(/Could not authorize a mentioned agent/),
  ).toBeVisible();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
  expect(await forumAdds(page)).toEqual([]);
  expect(await sent(page)).toEqual([]);
});

test("standalone forum still requires final authorization after a successful add", async ({
  page,
}) => {
  const dialog = await openStandaloneForumInvite(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.addChannelMembersDelayMs = 1_000;
  });
  await dialog.getByRole("button", { name: "Invite", exact: true }).click();
  await expect.poll(async () => (await forumAdds(page)).length).toBe(1);
  // Preparation already passed; revoke during the add, before final publish.
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentRevalidationRevokedPubkeys = [pubkey];
  }, REMOTE);
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByText(/Could not authorize a mentioned agent/),
  ).toBeVisible();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
  expect(await sent(page)).toEqual([]);
});

test("navigation during an outstanding forum Invite never publishes to either channel", async ({
  page,
}) => {
  const dialog = await openStandaloneForumInvite(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.addChannelMembersDelayMs = 800;
  });
  await dialog.getByRole("button", { name: "Invite", exact: true }).click();
  await expect.poll(async () => (await forumAdds(page)).length).toBe(1);
  // Route navigation unmounts the composer even while its modal traps focus.
  await page.evaluate(() => {
    window.location.hash = "/channels/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
  });
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForTimeout(1000);
  expect(await sent(page)).toEqual([]);
});
