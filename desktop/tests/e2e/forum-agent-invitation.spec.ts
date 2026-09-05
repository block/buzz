import { waitForAnimations } from "../helpers/animations";
import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const OWNER = "deadbeef".repeat(8);
const REMOTE = "ed".repeat(32);

async function install(
  page: Page,
  overrides: Parameters<typeof installMockBridge>[1] = {},
) {
  await installMockBridge(page, {
    ...overrides,
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
  await expect(
    dialog.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
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
  await expect(page.getByTestId("message-input")).toBeFocused();
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
    await expect(dialog.getByRole("alert")).toHaveText(error);
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/forum-error-${error.split(" ")[0].replace(/[^a-zA-Z0-9_-]/g, "-")}.png`,
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

// Hold the actual IPC boundary rather than rely on wall-clock sleeps.
type ForumGateWindow = Window & {
  __TAURI_INTERNALS__: {
    invoke: (command: string, payload?: unknown) => Promise<unknown>;
  };
  forumGateEntered?: boolean;
  forumGateDone?: boolean;
  releaseForumGate?: () => void;
};
async function holdForumCommand(
  page: Page,
  heldCommand: string,
  skip = 0,
  failure?: string,
) {
  await page.evaluate(
    ({ heldCommand, skip, failure }) => {
      const state = window as unknown as ForumGateWindow;
      const invoke = state.__TAURI_INTERNALS__.invoke;
      state.forumGateEntered = false;
      state.forumGateDone = false;
      const gate = new Promise<void>((resolve) => {
        state.releaseForumGate = resolve;
      });
      state.__TAURI_INTERNALS__.invoke = async (command, payload) => {
        if (command !== heldCommand || skip-- > 0)
          return invoke(command, payload);
        state.__TAURI_INTERNALS__.invoke = invoke;
        state.forumGateEntered = true;
        await gate;
        try {
          if (failure) throw new Error(failure);
          return await invoke(command, payload);
        } finally {
          state.forumGateDone = true;
        }
      };
    },
    { heldCommand, skip, failure },
  );
}
async function waitForForumGate(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as ForumGateWindow).forumGateEntered,
      ),
    )
    .toBe(true);
}
async function releaseForumGate(page: Page) {
  await page.evaluate(() =>
    (window as unknown as ForumGateWindow).releaseForumGate?.(),
  );
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as ForumGateWindow).forumGateDone),
    )
    .toBe(true);
}

for (const stage of ["add", "publish"] as const) {
  for (const replacement of [null, "new A draft", ""] as const) {
    test(`forum source visit ${stage}: A→B→A retains source draft; replacement=${replacement}`, async ({
      page,
    }) => {
      await install(page, {
        deferredComposerUploads: true,
        uploadDescriptors: [
          {
            url: `https://mock.relay/media/${"f".repeat(64)}.pdf`,
            sha256: "f".repeat(64),
            size: 12345,
            type: "application/pdf",
            uploaded: Math.floor(Date.now() / 1000),
            filename: "source.pdf",
          },
        ],
      });
      await page.getByTestId("channel-watercooler").click();
      await expect
        .poll(() =>
          page.evaluate(() =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "watercooler",
            }),
          ),
        )
        .toBe(true);
      const roots = await page.evaluate(() =>
        ["Source forum A", "Source forum B"].map(
          (content) =>
            window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
              channelName: "watercooler",
              content,
              kind: 45001,
            })?.id,
        ),
      );
      expect(roots.every(Boolean)).toBe(true);
      const navigate = async (index: number) => {
        await page.evaluate(
          ({ forum, id }) => {
            window.location.hash = `/channels/${forum}/posts/${id}`;
          },
          { forum: FORUM, id: roots[index] },
        );
        await expect(
          page.getByText(index === 0 ? "Source forum A" : "Source forum B", {
            exact: true,
          }),
        ).toBeVisible();
        await expect(page.getByTestId("message-input")).toHaveAttribute(
          "contenteditable",
          "true",
        );
      };
      // Cache both threads before the critical transition, so query loading
      // cannot accidentally supply the missing source-visit boundary.
      await navigate(1);
      await page.getByTestId("message-input").fill("B owns this draft");
      await navigate(0);
      await select(page);
      if (replacement === "new A draft") {
        await page.getByRole("button", { name: "Attach file" }).click();
        await expect(
          page.getByRole("button", { name: "Remove attachment" }),
        ).toBeVisible();
      }
      await page.getByTestId("send-message").click();
      await holdForumCommand(
        page,
        stage === "add" ? "add_channel_members" : "revalidate_relay_agents",
        stage === "publish" ? 1 : 0,
      );
      await page.getByRole("button", { name: "Invite", exact: true }).click();
      await waitForForumGate(page);
      await navigate(1);
      await expect(page.getByRole("alertdialog")).toHaveCount(0);
      await expect(page.getByTestId("message-input")).toHaveText(
        "B owns this draft",
      );
      await navigate(0);
      await expect(page.getByTestId("message-input")).toHaveText(
        "@RemoteScout hello",
      );
      if (replacement === "new A draft")
        await expect(
          page.getByRole("button", { name: "Remove attachment" }),
        ).toBeVisible();
      if (replacement !== null)
        await page.getByTestId("message-input").fill(replacement);
      await navigate(1);
      await releaseForumGate(page);
      expect(
        await page.evaluate(() =>
          (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).filter(
            (event) => event.kind === 45001 || event.kind === 45003,
          ),
        ),
      ).toEqual([]);
      expect(await sent(page)).toEqual([]);
      await expect(page.getByTestId("message-input")).toHaveText(
        "B owns this draft",
      );
      await navigate(0);
      await expect(page.getByTestId("message-input")).toHaveText(
        replacement ?? "@RemoteScout hello",
      );
      if (replacement === null) {
        // No re-selection: a recovered selected identity must still route exactly.
        await page.getByTestId("send-message").click();
        const dialog = page.getByRole("alertdialog");
        if (await dialog.count())
          await dialog
            .getByRole("button", { name: "Invite", exact: true })
            .click();
        await expect.poll(() => sent(page)).toEqual([[REMOTE]]);
      }
      await assertNoLocalLifecycle(page);
    });
  }
}

test("forum Escape cancels a pending add, restores focus, and a late add cannot publish", async ({
  page,
}) => {
  const dialog = await openStandaloneForumInvite(page);
  await holdForumCommand(page, "add_channel_members");
  await dialog.getByRole("button", { name: "Invite", exact: true }).click();
  await waitForForumGate(page);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("message-input")).toBeFocused();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
  await releaseForumGate(page);
  expect(await sent(page)).toEqual([]);
});

test("shared chat dialog keeps reference-only action and restores composer focus after Escape", async ({
  page,
}) => {
  await install(page);
  await select(page);
  await page.getByTestId("send-message").click();
  const dialog = page.getByRole("alertdialog");
  await expect(
    dialog.getByRole("button", { name: "Do nothing", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("message-input")).toBeFocused();
  await expect(page.getByTestId("message-input")).toHaveText(
    "@RemoteScout hello",
  );
  await page.getByTestId("send-message").click();
  await dialog.getByRole("button", { name: "Do nothing", exact: true }).click();
  await expect.poll(() => sent(page)).toEqual([[]]);
  expect(await forumAdds(page)).toEqual([]);
});

for (const replacement of [null, "new A draft", ""] as const) {
  test(`forum failed dispatched reply A→B→A recovers only without newer intent: ${replacement}`, async ({
    page,
  }) => {
    await install(page, {
      deferredComposerUploads: true,
      uploadDescriptors: [
        {
          url: `https://mock.relay/media/${"f".repeat(64)}.pdf`,
          sha256: "f".repeat(64),
          size: 12345,
          type: "application/pdf",
          uploaded: Math.floor(Date.now() / 1000),
          filename: "source.pdf",
        },
      ],
    });
    await page.getByTestId("channel-watercooler").click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "watercooler",
          }),
        ),
      )
      .toBe(true);
    const roots = await page.evaluate(() =>
      ["Transport A", "Transport B"].map(
        (content) =>
          window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName: "watercooler",
            content,
            kind: 45001,
          })?.id,
      ),
    );
    expect(roots.every(Boolean)).toBe(true);
    const navigate = async (index: number) => {
      await page.evaluate(
        ({ forum, id }) => {
          window.location.hash = `/channels/${forum}/posts/${id}`;
        },
        { forum: FORUM, id: roots[index] },
      );
      await expect(
        page.getByText(index === 0 ? "Transport A" : "Transport B", {
          exact: true,
        }),
      ).toBeVisible();
      await expect(page.getByTestId("message-input")).toHaveAttribute(
        "contenteditable",
        "true",
      );
    };
    await navigate(1);
    await page.getByTestId("message-input").fill("B owns this draft");
    await navigate(0);
    await select(page);
    await page.getByRole("button", { name: "Attach file" }).click();
    await expect(
      page.getByRole("button", { name: "Remove attachment" }),
    ).toBeVisible();
    await holdForumCommand(
      page,
      "send_channel_message",
      0,
      "transport offline",
    );
    await page.getByTestId("send-message").click();
    await page.getByRole("button", { name: "Invite", exact: true }).click();
    await waitForForumGate(page);
    await navigate(1);
    await expect(page.getByTestId("message-input")).toHaveText(
      "B owns this draft",
    );
    await navigate(0);
    await expect(page.getByTestId("message-input")).toHaveText("");
    if (replacement !== null) {
      // An actual edit -> delete is authoritative, not an unchanged empty editor.
      await page.getByTestId("message-input").fill("replacement intent");
      if (replacement === "") {
        await page.getByTestId("message-input").press("ControlOrMeta+a");
        await page.getByTestId("message-input").press("Backspace");
      } else {
        await page.getByTestId("message-input").fill(replacement);
      }
      await expect(page.getByTestId("message-input")).toHaveText(replacement);
    }
    await releaseForumGate(page);
    await expect(page.getByTestId("message-input")).toHaveText(
      replacement ?? "@RemoteScout hello",
    );
    await expect(
      page.getByRole("button", { name: "Remove attachment" }),
    ).toHaveCount(replacement === null ? 1 : 0);
    await navigate(1);
    await expect(page.getByTestId("message-input")).toHaveText(
      "B owns this draft",
    );
    await navigate(0);
    await expect(page.getByTestId("message-input")).toHaveText(
      replacement ?? "@RemoteScout hello",
    );
    // Recovery only restores authorship: no automatic send, add, or signing replay.
    expect(await forumAdds(page)).toHaveLength(1);
    const replies = () =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (call) =>
            call.command === "send_channel_message" &&
            (call.payload as { kind?: number })?.kind === 45003,
        ),
      );
    expect(await replies()).toHaveLength(0);
    if (replacement === null) {
      await page.getByTestId("send-message").click();
      await expect.poll(replies).toHaveLength(1);
      expect((await replies())[0].payload).toEqual(
        expect.objectContaining({
          channelId: FORUM,
          mentionPubkeys: [REMOTE],
          content: expect.stringContaining("@RemoteScout hello"),
          mediaTags: expect.arrayContaining([
            expect.arrayContaining([
              "imeta",
              `url https://mock.relay/media/${"f".repeat(64)}.pdf`,
            ]),
          ]),
        }),
      );
    }
    await assertNoLocalLifecycle(page);
  });
}
