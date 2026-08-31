import { waitForAnimations } from "../helpers/animations";
import { expect, test, type Page } from "@playwright/test";
import {
  installMockBridge,
  openNewMessagePage,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const OWNER = "deadbeef".repeat(8);
const REMOTE = "ed".repeat(32);
const GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

async function install(page: Page) {
  await installMockBridge(page, {
    ownerOnlyAccessBuild: true,
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
  await expect(row).toContainText("Remote Scout");
  await row.locator("button").first().click();
  await page.keyboard.type("hello");
}
async function sent(page: Page) {
  return page.evaluate(() => {
    const signed = (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
      .filter((event) => event.content === "@Remote Scout hello")
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
        payload.content === "@Remote Scout hello"
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
for (const role of ["member", "bot"] as const) {
  test(`owned ${role} with empty local roster emits exact p tag`, async ({
    page,
  }) => {
    await install(page);
    await page.evaluate(
      async ({ pubkey, channelId, role }) => {
        await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("add_channel_members", {
          channelId,
          pubkeys: [pubkey],
          role,
        });
        await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
          queryKey: ["channels"],
        });
        await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
          queryKey: ["relay-agents"],
        });
      },
      { pubkey: REMOTE, channelId: GENERAL, role },
    );
    await select(page);
    await page.getByTestId("send-message").click();
    await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
    await expect(
      page.getByRole("button", { name: "Invite", exact: true }),
    ).toHaveCount(0);
    await assertNoLocalLifecycle(page);
  });
}
test("owned nonmember uses authorized add before exact publication", async ({
  page,
}) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  const invite = page.getByRole("button", { name: "Invite", exact: true });
  await expect(invite).toBeVisible();
  expect(await sent(page)).toEqual([]);
  await invite.click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
  const calls = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
  );
  expect(calls).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "add_channel_members",
        payload: expect.objectContaining({ pubkeys: [REMOTE], role: "bot" }),
      }),
    ]),
  );
  await assertNoLocalLifecycle(page);
});
for (const error of [
  "actor not authorized",
  "policy:nobody — this agent has disabled external channel additions",
]) {
  test(`failed add keeps draft and sends nothing: ${error}`, async ({
    page,
  }) => {
    await install(page);
    await select(page);
    await page.evaluate((error) => {
      window.__BUZZ_E2E__.mock ??= {};
      window.__BUZZ_E2E__.mock.addChannelMembersErrors = [error];
    }, error);
    await page.getByTestId("send-message").click();
    await page.getByRole("button", { name: "Invite", exact: true }).click();
    await expect(page.getByText(error, { exact: true })).toBeVisible();
    await expect(page.getByTestId("message-input")).toHaveText(
      "@Remote Scout hello",
    );
    expect(await sent(page)).toEqual([]);
    await assertNoLocalLifecycle(page);
  });
}
test("selected owned agent revoked before add keeps draft and sends nothing", async ({
  page,
}) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentRevalidationRevokedPubkeys = [pubkey];
  }, REMOTE);
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(
    page.getByText(/Could not authorize a mentioned agent/),
  ).toBeVisible();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@Remote Scout hello",
  );
  expect(await sent(page)).toEqual([]);
});

for (const mode of ["existing", "new"] as const) {
  test(`${mode} DM prepares actual destination for owned relay mention`, async ({
    page,
  }) => {
    await install(page);
    if (mode === "existing") {
      await page.getByTestId("channel-bob-tyler").click();
      await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
    } else {
      await openNewMessagePage(page);
      await page.getByTestId("new-dm-search").fill("bob");
      await page
        .getByTestId(`new-dm-result-${TEST_IDENTITIES.bob.pubkey}`)
        .click();
      await page.getByTestId("new-dm-search").press("Escape");
    }
    await select(page);
    await page.getByTestId("send-message").click();
    await expect
      .poll(() => sent(page))
      .toEqual([[REMOTE, TEST_IDENTITIES.bob.pubkey]]);
    const calls = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
    );
    const checks = calls.filter(
      (call) => call.command === "revalidate_relay_agents",
    );
    const event = await page.evaluate(() =>
      (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).find(
        (event) => event.content === "@Remote Scout hello",
      ),
    );
    expect(checks.at(-1)?.payload).toMatchObject({
      channelId:
        event?.tags.find((tag) => tag[0] === "h")?.[1] ??
        (
          calls.find((call) => call.command === "send_channel_message")
            ?.payload as { channelId?: string }
        )?.channelId,
      pubkeys: [REMOTE],
    });
    await assertNoLocalLifecycle(page);
  });
}

test("membership revoked at final publish keeps draft and emits no message", async ({
  page,
}) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  // Let preparation succeed, but make the fresh final directory read fail.
  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentListErrors = [
      null,
      null,
      "revoked at publication",
    ];
  });
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await expect(
    page.getByText(/Could not authorize a mentioned agent/),
  ).toBeVisible();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@Remote Scout hello",
  );
  expect(await sent(page)).toEqual([]);
});

const FORUM = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";

async function openStandaloneForumInvite(page: Page) {
  await install(page);
  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await page.getByRole("button", { name: "Start a new post..." }).click();
  await select(page);
  await page.getByTestId("send-message").click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText("Remote Scout");
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
  // Unlike chat's explicit reference-only option, cancel must not publish.
  await expect(
    dialog.getByRole("button", { name: /Do nothing|Send anyway/ }),
  ).toHaveCount(0);
  await dialog.getByRole("button", { name: "Invite", exact: true }).click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
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
    "@Remote Scout hello",
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
    await expect(page.getByTestId("message-input")).toHaveText(
      "@Remote Scout hello",
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
    "@Remote Scout hello",
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
    "@Remote Scout hello",
  );
  expect(await sent(page)).toEqual([]);
});

test("cloud provenance stays consistent from picker through hover, profile and DM sidebar", async ({
  page,
}, testInfo) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
  const chip = page
    .locator("article")
    .filter({ hasText: "hello" })
    .getByRole("button", { name: /Remote Scout/ })
    .first();
  await expect(chip.locator("svg.lucide-cloud")).toBeVisible();
  await chip.hover();
  const hover = page.getByTestId("user-profile-popover");
  const marker = hover.getByTestId("user-profile-popover-agent-provenance");
  await expect(marker).toHaveAttribute("title", "Not managed on this device");
  await expect(marker.locator("svg.lucide-cloud")).toBeVisible();
  await waitForAnimations(page);
  await hover.screenshot({
    path: testInfo.outputPath("remote-cloud-hover.png"),
  });
  await chip.click();
  const panel = page.getByTestId("user-profile-panel");
  await expect(
    panel
      .getByRole("img", { name: "Not managed on this device" })
      .locator("svg.lucide-cloud"),
  ).toBeVisible();
  await waitForAnimations(page);
  await panel.screenshot({
    path: testInfo.outputPath("remote-cloud-profile.png"),
  });
  await page.getByTestId("auxiliary-panel-close").click();
  const dm = await page.evaluate(async (pubkey) => {
    const result = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("open_dm", {
      pubkeys: [pubkey],
    })) as { id: string };
    await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
      queryKey: ["channels"],
    });
    return result;
  }, REMOTE);
  const sidebarMarker = page.getByTestId(`channel-agent-provenance-${dm.id}`);
  await expect(sidebarMarker.locator("svg.lucide-cloud")).toBeVisible();
  await page.locator(`[data-channel-id="${dm.id}"]`).click();
  await expect(
    page
      .getByTestId("chat-header-agent-provenance")
      .locator("svg.lucide-cloud"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: testInfo.outputPath("remote-cloud-dm.png") });
});
