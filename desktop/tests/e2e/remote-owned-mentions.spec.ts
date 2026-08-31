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
  await waitForAnimations(page);
  await page.screenshot({ path: "test-results/remote-invite.png" });
  expect(await sent(page)).toEqual([]);
  await invite.click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
  await waitForAnimations(page);
  await page.screenshot({ path: "test-results/remote-sent.png" });
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
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/remote-error-${error.split(" ")[0].replace(/[^a-zA-Z0-9_-]/g, "-")}.png`,
    });
    await expect(page.getByTestId("message-input")).toHaveText(
      "@RemoteScout hello",
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
    "@RemoteScout hello",
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
        (event) => event.content === "@RemoteScout hello",
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
    "@RemoteScout hello",
  );
  expect(await sent(page)).toEqual([]);
});

// Deferred IPC seam: hold the exact next preparation/add response, not a timer.
// This lets the browser exercise Escape/navigation before the continuation runs.
type InviteGateWindow = Window & {
  __TAURI_INTERNALS__: {
    invoke: (command: string, payload?: unknown) => Promise<unknown>;
  };
  inviteGateEntered?: boolean;
  releaseInviteGate?: () => void;
};
async function holdInviteCommand(page: Page, command: string, skip = 0) {
  await page.evaluate(
    ({ heldCommand, skip }) => {
      const state = window as unknown as InviteGateWindow;
      const invoke = state.__TAURI_INTERNALS__.invoke;
      const gate = new Promise<void>((resolve) => {
        state.releaseInviteGate = resolve;
      });
      state.__TAURI_INTERNALS__.invoke = async (command, payload) => {
        if (command !== heldCommand || skip-- > 0)
          return invoke(command, payload);
        state.__TAURI_INTERNALS__.invoke = invoke;
        state.inviteGateEntered = true;
        await gate;
        return invoke(command, payload);
      };
    },
    { heldCommand: command, skip },
  );
}
async function releaseInviteCommand(page: Page) {
  await page.evaluate(() => {
    (window as unknown as InviteGateWindow).releaseInviteGate?.();
  });
}
async function waitForInviteGate(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as InviteGateWindow).inviteGateEntered,
      ),
    )
    .toBe(true);
}
async function remoteAdds(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
      (call) => call.command === "add_channel_members",
    ),
  );
}
test("B1 delayed preparation shows pending; Escape retains draft and cancels add/send", async ({
  page,
}) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  await holdInviteCommand(page, "revalidate_relay_agents");
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await waitForInviteGate(page);
  await expect(
    page.getByRole("button", { name: "Inviting...", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Do nothing", exact: true }),
  ).toBeDisabled();
  await waitForAnimations(page);
  await page
    .getByRole("alertdialog")
    .screenshot({ path: "test-results/b1-invite-pending.png" });
  await page.keyboard.press("Escape");
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await releaseInviteCommand(page);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "revalidate_relay_agents",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  await page.waitForTimeout(300);
  expect(await remoteAdds(page)).toEqual([]);
  expect(await sent(page)).toEqual([]);
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
  // A retry is a new intent and still succeeds exactly once.
  await page.getByTestId("send-message").click();
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
});
test("B1 navigation during delayed add cannot publish its captured draft", async ({
  page,
}) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  await holdInviteCommand(page, "add_channel_members");
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await waitForInviteGate(page);
  // Hash routing unmounts the composer without reloading the IPC context.
  await page.evaluate(() => {
    window.location.hash = "/channels/9dae0116-799b-5071-a0a8-fdd30a91a35d";
  });
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await releaseInviteCommand(page);
  await expect.poll(async () => (await remoteAdds(page)).length).toBe(1);
  await page.waitForTimeout(300);
  expect(await sent(page)).toEqual([]);
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
});

for (const stage of ["add", "publish"] as const) {
  for (const incoming of ["unrelated thread B draft", "@RemoteScout hello"]) {
    test(`B1 same-channel thread navigation during ${stage} preserves ${incoming}`, async ({
      page,
    }) => {
      await install(page);
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
                channelName: "general",
              }) ?? false,
          ),
        )
        .toBe(true);
      const roots = await page.evaluate(() =>
        ["Lifecycle thread A", "Lifecycle thread B"].map(
          (content) =>
            window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
              channelName: "general",
              content,
            })?.id,
        ),
      );
      expect(roots.every(Boolean)).toBe(true);
      const navigate = async (id: string | undefined) => {
        // Drive the real reply route handler, including while the modal has
        // focus (the equivalent external history/navigation intent).
        await page
          .getByTestId(`reply-message-${id}`)
          .first()
          .evaluate((button) => (button as HTMLButtonElement).click());
        await expect(page.getByTestId("message-thread-panel")).toBeVisible();
        await expect(page.getByTestId("message-thread-head")).toContainText(
          id === roots[0] ? "Lifecycle thread A" : "Lifecycle thread B",
        );
      };
      const input = page
        .getByTestId("message-thread-panel")
        .getByTestId("message-input");
      // Visit both first: the transition under test must reuse the available
      // panel/composer, not get a fortuitous unmount via a loading skeleton.
      await navigate(roots[1]);
      await input.fill(incoming);
      await navigate(roots[0]);
      await expect(input).toHaveText("");
      await input.fill("@Remote");
      await page
        .getByTestId(`mention-suggestion-${REMOTE}`)
        .locator("button")
        .first()
        .click();
      await page.keyboard.type("hello");
      await page
        .getByTestId("message-thread-panel")
        .getByTestId("send-message")
        .click();
      await holdInviteCommand(
        page,
        stage === "add" ? "add_channel_members" : "revalidate_relay_agents",
        stage === "publish" ? 2 : 0,
      );
      await page.getByRole("button", { name: "Invite", exact: true }).click();
      await waitForInviteGate(page);
      // Expando proves the actual editor DOM host survived A -> B.
      await input.evaluate((el) =>
        el.setAttribute("data-lifecycle-host", "retained"),
      );
      await navigate(roots[1]);
      await expect(input).toHaveAttribute("data-lifecycle-host", "retained");
      await expect(input).toHaveText(incoming);
      await expect(page.getByRole("alertdialog")).toHaveCount(0);
      // Return before resolution: optimistic recovery must already be durable.
      await navigate(roots[0]);
      await expect(input).toHaveText("@RemoteScout hello");
      await input.fill("new A draft after return");
      await navigate(roots[1]);
      await releaseInviteCommand(page);
      await page.waitForTimeout(300);
      expect(await sent(page)).toEqual([]);
      await expect(input).toHaveText(incoming);
      await navigate(roots[0]);
      await expect(input).toHaveText("new A draft after return");
      await assertNoLocalLifecycle(page);
    });
  }
}

for (const incoming of ["unrelated thread B draft", "@RemoteScout hello"]) {
  test(`B1 authored deletion before thread switch preserves storage and ${incoming}`, async ({
    page,
  }) => {
    await install(page);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "general",
            }) ?? false,
        ),
      )
      .toBe(true);
    const roots = await page.evaluate(() =>
      ["Lifecycle thread A", "Lifecycle thread B"].map(
        (content) =>
          window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName: "general",
            content,
          })?.id,
      ),
    );
    expect(roots.every(Boolean)).toBe(true);
    const navigate = async (id: string | undefined) => {
      // Drive the real reply route handler, including while the modal has
      // focus (the equivalent external history/navigation intent).
      await page
        .getByTestId(`reply-message-${id}`)
        .first()
        .evaluate((button) => (button as HTMLButtonElement).click());
      await expect(page.getByTestId("message-thread-panel")).toBeVisible();
      await expect(page.getByTestId("message-thread-head")).toContainText(
        id === roots[0] ? "Lifecycle thread A" : "Lifecycle thread B",
      );
    };
    const input = page
      .getByTestId("message-thread-panel")
      .getByTestId("message-input");
    // Visit both first: the transition under test must reuse the available
    // panel/composer, not get a fortuitous unmount via a loading skeleton.
    await navigate(roots[1]);
    await input.fill(incoming);
    await navigate(roots[0]);
    await expect(input).toHaveText("");
    await input.fill("@Remote");
    await page
      .getByTestId(`mention-suggestion-${REMOTE}`)
      .locator("button")
      .first()
      .click();
    await page.keyboard.type("hello");
    await page
      .getByTestId("message-thread-panel")
      .getByTestId("send-message")
      .click();
    await holdInviteCommand(page, "revalidate_relay_agents", 2);
    await page.getByRole("button", { name: "Invite", exact: true }).click();
    await waitForInviteGate(page);
    await expect(input).toHaveText("");
    await input.fill("new authored text");
    await input.fill("");
    const sourceRecord = () =>
      page.evaluate(([root, otherRoot]) => {
        const key = Object.keys(localStorage).find((key) =>
          key.startsWith("buzz-drafts.v2"),
        );
        if (!key) throw new Error("draft storage scope missing");
        const drafts = JSON.parse(localStorage.getItem(key) ?? "{}");
        if (!drafts[`thread:${otherRoot}`])
          throw new Error("control B draft missing");
        return drafts[`thread:${root}`] ?? null;
      }, roots);
    expect(await sourceRecord()).toBeNull();
    // Expando proves the actual editor DOM host survived A -> B.
    await input.evaluate((el) =>
      el.setAttribute("data-lifecycle-host", "retained"),
    );
    await navigate(roots[1]);
    await expect(input).toHaveAttribute("data-lifecycle-host", "retained");
    await expect(input).toHaveText(incoming);
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
    // Read actual persistence, not the editor (whose tombstone could mask a
    // resurrected record until reload). Neither text nor exact refs may return.
    expect(await sourceRecord()).toBeNull();
    await navigate(roots[0]);
    await expect(input).toHaveText("");

    await navigate(roots[1]);
    await releaseInviteCommand(page);
    await page.waitForTimeout(300);
    expect(await sent(page)).toEqual([]);
    await expect(input).toHaveText(incoming);
    await navigate(roots[0]);
    await expect(input).toHaveText("");
    expect(await sourceRecord()).toBeNull();
    await assertNoLocalLifecycle(page);
  });
}

for (const incoming of ["B preserved"]) {
  test(`ordinary failure after returning to A and deleting does not resurrect storage`, async ({
    page,
  }) => {
    await install(page);
    await page.evaluate(
      async ({ pubkey, channelId }) => {
        await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("add_channel_members", {
          channelId,
          pubkeys: [pubkey],
          role: "bot",
        });
        await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
          queryKey: ["channels"],
        });
        await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
          queryKey: ["relay-agents"],
        });
      },
      { pubkey: REMOTE, channelId: GENERAL },
    );
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "general",
            }) ?? false,
        ),
      )
      .toBe(true);
    const roots = await page.evaluate(() =>
      ["Lifecycle thread A", "Lifecycle thread B"].map(
        (content) =>
          window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName: "general",
            content,
          })?.id,
      ),
    );
    expect(roots.every(Boolean)).toBe(true);
    const navigate = async (id: string | undefined) => {
      // Drive the real reply route handler, including while the modal has
      // focus (the equivalent external history/navigation intent).
      await page
        .getByTestId(`reply-message-${id}`)
        .first()
        .evaluate((button) => (button as HTMLButtonElement).click());
      await expect(page.getByTestId("message-thread-panel")).toBeVisible();
      await expect(page.getByTestId("message-thread-head")).toContainText(
        id === roots[0] ? "Lifecycle thread A" : "Lifecycle thread B",
      );
    };
    const input = page
      .getByTestId("message-thread-panel")
      .getByTestId("message-input");
    // Visit both first: the transition under test must reuse the available
    // panel/composer, not get a fortuitous unmount via a loading skeleton.
    await navigate(roots[1]);
    await input.fill(incoming);
    await navigate(roots[0]);
    await expect(input).toHaveText("");
    await input.fill("@Remote");
    await page
      .getByTestId(`mention-suggestion-${REMOTE}`)
      .locator("button")
      .first()
      .click();
    await page.keyboard.type("hello");
    await holdInviteCommand(page, "revalidate_relay_agents", 1);
    await page
      .getByTestId("message-thread-panel")
      .getByTestId("send-message")
      .click();
    await waitForInviteGate(page);
    await expect(input).toHaveText("");
    await input.evaluate((el) =>
      el.setAttribute("data-lifecycle-host", "retained"),
    );
    await navigate(roots[1]);
    await expect(input).toHaveAttribute("data-lifecycle-host", "retained");
    await expect(input).toHaveText(incoming);
    await navigate(roots[0]);
    await expect(input).toHaveAttribute("data-lifecycle-host", "retained");
    await input.fill("new authored text");
    await input.fill("");
    const sourceRecord = () =>
      page.evaluate(([root, otherRoot]) => {
        const key = Object.keys(localStorage).find((key) =>
          key.startsWith("buzz-drafts.v2"),
        );
        if (!key) throw new Error("draft storage scope missing");
        const drafts = JSON.parse(localStorage.getItem(key) ?? "{}");
        if (!drafts[`thread:${otherRoot}`])
          throw new Error("control B draft missing");
        return drafts[`thread:${root}`] ?? null;
      }, roots);
    expect(await sourceRecord()).toBeNull();
    // Expando proves the actual editor DOM host survived A -> B.
    await input.evaluate((el) =>
      el.setAttribute("data-lifecycle-host", "retained"),
    );
    await navigate(roots[1]);
    await expect(input).toHaveAttribute("data-lifecycle-host", "retained");
    await expect(input).toHaveText(incoming);
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
    // Read actual persistence, not the editor (whose tombstone could mask a
    // resurrected record until reload). Neither text nor exact refs may return.
    expect(await sourceRecord()).toBeNull();
    await navigate(roots[0]);
    await expect(input).toHaveText("");

    await navigate(roots[1]);
    await page.evaluate((pubkey) => {
      window.__BUZZ_E2E__.mock ??= {};
      window.__BUZZ_E2E__.mock.relayAgentRevalidationRevokedPubkeys = [pubkey];
    }, REMOTE);
    await releaseInviteCommand(page);
    await expect(
      page.getByText(/Could not authorize a mentioned agent/),
    ).toBeVisible();
    await page.waitForTimeout(300);
    expect(await sent(page)).toEqual([]);
    await expect(input).toHaveText(incoming);
    await navigate(roots[0]);
    await expect(input).toHaveText("");
    expect(await sourceRecord()).toBeNull();
    await assertNoLocalLifecycle(page);
  });
}
