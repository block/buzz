import { expect, type Page, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

type MockMessageWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    createdAt?: number;
    pubkey?: string;
  }) => { id: string } | undefined;
  __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
    channelName: string;
    kind?: number;
  }) => boolean;
  __BUZZ_E2E_EMIT_MOCK_TYPING__?: (input: {
    channelName: string;
    pubkey?: string;
    threadHeadId?: string;
  }) => unknown;
  __BUZZ_E2E_SET_REMOTE_THREAD_TITLE__?: (input: {
    channelName: string;
    rootId: string;
    title: string;
  }) => void;
  __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
    agentPubkey: string;
    channelId: string;
    turnId: string;
    kind?: "turn_started" | "turn_completed";
    threadRootEventId?: string;
  }) => void;
};

const CHANNEL_NAME = "engineering";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

async function openThread(
  page: Page,
  rootContent: string,
  options: {
    channelName?: string;
    replyCreatedAt?: number;
    replyPubkey?: string;
  } = {},
): Promise<string> {
  const channelName = options.channelName ?? CHANNEL_NAME;
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (
            window as MockMessageWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }) ??
          false,
        channelName,
      ),
    )
    .toBe(true);

  const summariesBefore = await page
    .getByTestId("message-thread-summary")
    .count();
  const rootId = await page.evaluate(
    ({ channelName, content, pubkey }) =>
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content,
        pubkey,
      })?.id ?? null,
    {
      channelName,
      content: rootContent,
      pubkey: MOCK_IDENTITY_PUBKEY,
    },
  );
  expect(rootId).not.toBeNull();
  await page.evaluate(
    ({ channelName, createdAt, parentEventId, pubkey }) => {
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content: "First reply.",
        createdAt,
        parentEventId,
        pubkey,
      });
    },
    {
      channelName,
      createdAt: options.replyCreatedAt,
      parentEventId: rootId,
      pubkey: options.replyPubkey ?? ALICE_PUBKEY,
    },
  );
  // A new root collapses its reply into a summary row as it renders, so a
  // click on the inline reply can land on a detached node. Wait for the
  // summary instead; `.last()` alone could still resolve to an earlier one.
  const summaries = page.getByTestId("message-thread-summary");
  await expect(summaries).toHaveCount(summariesBefore + 1);
  await summaries.last().click();
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(rootContent);
  return rootId as string;
}

async function emitReply(page: Page, parentEventId: string, content: string) {
  await page.evaluate(
    ({ channelName, content, parentEventId, pubkey }) => {
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content,
        parentEventId,
        pubkey,
      });
    },
    { channelName: CHANNEL_NAME, content, parentEventId, pubkey: ALICE_PUBKEY },
  );
}

async function openReplyMenu(page: Page, content: string) {
  const panel = page.getByTestId("message-thread-panel");
  const replyId = await panel
    .locator("[data-message-id]")
    .filter({ hasText: content })
    .last()
    .getAttribute("data-message-id");
  const toggle = page.getByTestId(`mark-read-toggle-${replyId}`);
  // A click while the previous menu is still closing can be swallowed.
  await expect(async () => {
    await panel.locator(`[data-message-id="${replyId}"]`).hover();
    await page.getByTestId(`more-actions-${replyId}`).click();
    await expect(toggle).toBeVisible({ timeout: 1_000 });
  }).toPass();
  return toggle;
}

async function nameOpenThread(page: Page, title: string) {
  const panel = page.getByTestId("message-thread-panel");
  await panel.getByTestId("thread-title").hover();
  await panel.getByTestId("thread-title-edit").click();
  await panel.getByTestId("thread-title-input").fill(title);
  await panel.getByTestId("thread-title-input").press("Enter");
  await expect(panel.getByTestId("thread-title")).toHaveText(title);
}

/**
 * Sets titles as another user would. The title list is cached app-wide and
 * only a title event in a live-subscribed channel invalidates it, so the last
 * title must be in CHANNEL_NAME; that write is what makes the others show.
 */
async function setRemoteTitles(
  page: Page,
  titles: { channelName: string; rootId: string; title: string }[],
) {
  expect(titles.at(-1)?.channelName).toBe(CHANNEL_NAME);
  await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (
            window as MockMessageWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
            kind: 45010,
          }) ?? false,
        CHANNEL_NAME,
      ),
    )
    .toBe(true);
  await page.evaluate((titles) => {
    for (const title of titles) {
      (window as MockMessageWindow).__BUZZ_E2E_SET_REMOTE_THREAD_TITLE__?.(
        title,
      );
    }
  }, titles);
}

test.describe("thread titles", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("name a thread, find it in the Threads view, and reopen it", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await openThread(page, "Root message that will get a title.");

    const panel = page.getByTestId("message-thread-panel");
    await expect(panel.getByTestId("thread-title")).toHaveText("Thread");

    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    const input = panel.getByTestId("thread-title-input");
    await expect(input).toBeFocused();
    await input.fill("  Release checklist  ");
    await input.press("Enter");
    await expect(panel.getByTestId("thread-title")).toHaveText(
      "Release checklist",
    );
    // The root message in the channel carries the title on its own line, and
    // the thread panel's copy of the root doesn't repeat it under the header.
    const titleLine = page.getByTestId("thread-title-line");
    await expect(titleLine).toHaveCount(1);
    await expect(titleLine).toHaveText("Release checklist");
    await expect(panel.getByTestId("thread-title-line")).toHaveCount(0);

    // Escape abandons an edit without writing.
    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    await panel.getByTestId("thread-title-input").fill("Discarded");
    await panel.getByTestId("thread-title-input").press("Escape");
    await expect(panel.getByTestId("thread-title")).toHaveText(
      "Release checklist",
    );

    await page.getByTestId("open-threads-view").click();
    const rows = page.getByTestId("threads-view-row");
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("Release checklist");
    await expect(rows.first()).toContainText(`#${CHANNEL_NAME}`);

    // The thread opens beside the list; the Threads view stays put.
    await rows.first().click();
    const sidePanel = page.getByTestId("message-thread-panel");
    await expect(
      sidePanel.getByTestId("message-thread-open-channel"),
    ).toHaveText(`Release checklist · #${CHANNEL_NAME}`);
    await expect(page.getByTestId("threads-view")).toBeVisible();
    await expect(rows.first()).toHaveAttribute("data-selected", "true");

    // Like the Inbox header, the title is a link, not an editor: it opens
    // the thread in its channel, where it can still be renamed.
    await expect(sidePanel.getByTestId("thread-title-edit")).toHaveCount(0);
    await sidePanel.getByRole("button", { name: "Open full thread" }).click();
    await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL_NAME);
    await expect(page.getByTestId("threads-view")).toHaveCount(0);
    await expect(
      page.getByTestId("message-thread-panel").getByTestId("thread-title"),
    ).toHaveText("Release checklist");
    await expect(
      page.getByTestId("message-thread-panel").getByTestId("thread-title-edit"),
    ).toHaveCount(1);
  });

  test("own messages can be edited and deleted from the Threads view", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    const rootId = await openThread(page, "Root of a thread to tidy up.");
    await nameOpenThread(page, "Tidy up");
    const [editId, deleteId] = await page.evaluate(
      ({ channelName, parentEventId, pubkey }) =>
        ["My reply before editing.", "My reply to delete."].map(
          (content) =>
            (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
              channelName,
              content,
              parentEventId,
              pubkey,
            })?.id ?? null,
        ),
      {
        channelName: CHANNEL_NAME,
        parentEventId: rootId,
        pubkey: MOCK_IDENTITY_PUBKEY,
      },
    );
    expect(editId).not.toBeNull();
    expect(deleteId).not.toBeNull();

    await page.getByTestId("open-threads-view").click();
    await page.getByTestId("threads-view-row").first().click();
    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toContainText("My reply to delete.");

    const openMoreActions = async (messageId: string) => {
      await panel.locator(`[data-message-id="${messageId}"]`).hover();
      await page.getByTestId(`more-actions-${messageId}`).click();
      await expect(page.locator('[role="menuitem"]').first()).toBeVisible();
    };

    // Someone else's reply gets neither action.
    const foreignReplyId = await panel
      .locator("[data-message-id]")
      .filter({ hasText: "First reply." })
      .last()
      .getAttribute("data-message-id");
    await openMoreActions(foreignReplyId as string);
    await expect(
      page.getByTestId(`delete-message-${foreignReplyId}`),
    ).toHaveCount(0);
    await expect(
      page.getByTestId(`edit-message-${foreignReplyId}`),
    ).toHaveCount(0);
    await page.keyboard.press("Escape");

    await openMoreActions(editId as string);
    await page.getByTestId(`edit-message-${editId}`).click();
    await expect(panel.getByTestId("edit-target")).toBeVisible();
    await panel.getByTestId("message-input").click();
    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.type("My reply after editing.");
    await page.keyboard.press("Enter");
    await expect(panel.getByTestId("edit-target")).toBeHidden();
    await expect(panel.locator(`[data-message-id="${editId}"]`)).toContainText(
      "My reply after editing.",
    );

    await openMoreActions(deleteId as string);
    await page.getByTestId(`delete-message-${deleteId}`).click();
    await page
      .getByRole("alertdialog")
      .getByRole("button", { name: "Delete" })
      .click();
    await expect(panel).not.toContainText("My reply to delete.");
    await expect(page.getByTestId("threads-view")).toBeVisible();

    // Deleting the root leaves nothing to show, so the pane closes.
    await openMoreActions(rootId);
    await page.getByTestId(`delete-message-${rootId}`).click();
    await page
      .getByRole("alertdialog")
      .getByRole("button", { name: "Delete" })
      .click();
    await expect(panel).toHaveCount(0);
    await expect(page.getByTestId("threads-view")).toBeVisible();
  });

  test("ArrowUp in the Threads view composer edits your last message", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    const now = Math.floor(Date.now() / 1000);
    const rootId = await openThread(page, "Root before my last reply.", {
      replyCreatedAt: now + 120,
    });
    await nameOpenThread(page, "Up to edit");
    // Newer than the root but older than Alice's reply, so ArrowUp has to
    // skip someone else's newer message and pick this one over the root.
    const ownReplyId = await page.evaluate(
      ({ channelName, createdAt, parentEventId, pubkey }) =>
        (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: "My last reply.",
          createdAt,
          parentEventId,
          pubkey,
        })?.id ?? null,
      {
        channelName: CHANNEL_NAME,
        createdAt: now + 60,
        parentEventId: rootId,
        pubkey: MOCK_IDENTITY_PUBKEY,
      },
    );
    expect(ownReplyId).not.toBeNull();

    await page.getByTestId("open-threads-view").click();
    await page.getByTestId("threads-view-row").first().click();
    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toContainText("My last reply.");

    await panel.getByTestId("message-input").click();
    await page.keyboard.press("ArrowUp");
    await expect(panel.getByTestId("edit-target")).toBeVisible();
    await expect(panel.getByTestId("message-input")).toHaveText(
      "My last reply.",
    );
    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.type("My last reply, edited.");
    await page.keyboard.press("Enter");
    await expect(panel.getByTestId("edit-target")).toBeHidden();
    await expect(
      panel.locator(`[data-message-id="${ownReplyId}"]`),
    ).toContainText("My last reply, edited.");
  });

  test("clearing the title removes the thread from the Threads view", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await openThread(page, "Root message that is named, then cleared.");

    const panel = page.getByTestId("message-thread-panel");
    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    await panel.getByTestId("thread-title-input").fill("Temporary name");
    await panel.getByTestId("thread-title-input").press("Enter");
    await expect(panel.getByTestId("thread-title")).toHaveText(
      "Temporary name",
    );

    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    await panel.getByTestId("thread-title-input").fill("   ");
    await panel.getByTestId("thread-title-input").press("Enter");
    await expect(panel.getByTestId("thread-title")).toHaveText("Thread");

    await page.getByTestId("open-threads-view").click();
    await expect(page.getByTestId("threads-view")).toContainText(
      "No named threads",
    );
    await expect(page.getByTestId("threads-view-subheader")).toHaveText(
      "Named threads, most recent activity first. Name any thread from its header.",
    );
  });

  test("unread replies in a named thread show in the Threads view and sidebar", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    const rootId = await openThread(
      page,
      "Root message that gets new replies.",
    );
    await nameOpenThread(page, "Unread tracking");

    await page.getByTestId("open-threads-view").click();
    const row = page.getByTestId("threads-view-row");
    await expect(row).toHaveCount(1);
    await expect(page.getByTestId("threads-unread-dot")).toHaveCount(0);

    await emitReply(page, rootId, "A reply while you're away.");
    await emitReply(page, rootId, "And another one.");
    await expect(page.getByTestId("threads-unread-dot")).toBeVisible();
    await expect(row).toHaveAttribute("data-unread", "true");
    await expect(page.getByTestId("threads-view-row-unread")).toHaveText(
      "2 new",
    );

    // Opening the thread reads it, which clears both indicators.
    await row.click();
    await expect(
      page
        .getByTestId("message-thread-panel")
        .getByTestId("message-thread-open-channel"),
    ).toHaveText(`Unread tracking · #${CHANNEL_NAME}`);
    await expect(page.getByTestId("threads-unread-dot")).toHaveCount(0);
    await page.getByTestId("open-threads-view").click();
    await expect(row).not.toHaveAttribute("data-unread", "true");
  });

  test("mark a reply unread from the Threads view", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    const rootId = await openThread(page, "Root of a thread to revisit.");
    await nameOpenThread(page, "Revisit later");
    await emitReply(page, rootId, "A reply to come back to.");

    await page.getByTestId("open-threads-view").click();
    const row = page.getByTestId("threads-view-row");
    await row.click();
    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toContainText("A reply to come back to.");
    await expect(row).not.toHaveAttribute("data-unread", "true");

    const replyId = await panel
      .locator("[data-message-id]")
      .filter({ hasText: "A reply to come back to." })
      .last()
      .getAttribute("data-message-id");
    await panel.locator(`[data-message-id="${replyId}"]`).hover();
    await page.getByTestId(`more-actions-${replyId}`).click();
    await page.getByTestId(`mark-read-toggle-${replyId}`).click();

    // The open thread must not read the reply straight back.
    await expect(page.getByTestId("threads-view-row-unread")).toHaveText(
      "1 new",
    );
    await expect(page.getByTestId("threads-unread-dot")).toBeVisible();

    // Back on the list it is still unread; reopening the thread reads it.
    await page.getByTestId("open-threads-view").click();
    await expect(panel).toHaveCount(0);
    await expect(row).toHaveAttribute("data-unread", "true");
    await row.click();
    await expect(panel).toContainText("A reply to come back to.");
    await expect(row).not.toHaveAttribute("data-unread", "true");
    await expect(page.getByTestId("threads-unread-dot")).toHaveCount(0);
  });

  test("the Threads view pane marks where the new replies start", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    const rootId = await openThread(page, "Root of a thread with news.");
    await nameOpenThread(page, "News thread");

    await page.getByTestId("open-threads-view").click();
    const row = page.getByTestId("threads-view-row");
    // Until the channel's thread panel unmounts it reads new replies itself.
    await expect(row).toHaveCount(1);
    await emitReply(page, rootId, "The first new reply.");
    await emitReply(page, rootId, "The second new reply.");
    await expect(page.getByTestId("threads-view-row-unread")).toHaveText(
      "2 new",
    );

    // Opening reads the thread, but the divider still marks what was new.
    await row.click();
    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toContainText("The second new reply.");
    await expect(row).not.toHaveAttribute("data-unread", "true");
    const divider = panel.getByTestId("message-unread-divider");
    await expect(divider).toHaveCount(1);
    const top = async (text: string) =>
      (await panel.getByText(text, { exact: true }).boundingBox())?.y ?? 0;
    const dividerTop = (await divider.boundingBox())?.y ?? 0;
    expect(dividerTop).toBeGreaterThan(await top("First reply."));
    expect(dividerTop).toBeLessThan(await top("The first new reply."));
  });

  test("Mark read in the Threads view pane undoes Mark unread", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    const rootId = await openThread(page, "Root of a thread to read twice.");
    await nameOpenThread(page, "Read twice");
    await emitReply(page, rootId, "A reply to flag and unflag.");

    await page.getByTestId("open-threads-view").click();
    const row = page.getByTestId("threads-view-row");
    await row.click();
    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toContainText("A reply to flag and unflag.");

    const toggle = await openReplyMenu(page, "A reply to flag and unflag.");
    await expect(toggle).toHaveText("Mark unread");
    await toggle.click();
    await expect(page.getByTestId("threads-view-row-unread")).toHaveText(
      "1 new",
    );

    const sameToggle = await openReplyMenu(page, "A reply to flag and unflag.");
    await expect(sameToggle).toHaveText("Mark read");
    await sameToggle.click();
    await expect(row).not.toHaveAttribute("data-unread", "true");
    await expect(page.getByTestId("threads-unread-dot")).toHaveCount(0);

    // With nothing left marked unread, the open thread reads new replies again.
    await emitReply(page, rootId, "A reply after reading it all.");
    await expect(panel).toContainText("A reply after reading it all.");
    await expect(row).not.toHaveAttribute("data-unread", "true");
  });

  test("the Threads view pane shows who is typing and which agents are working", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    // Typing right after your own message is suppressed, so the agent must
    // not be the one who wrote the first reply.
    const rootId = await openThread(page, "Root of a thread with an agent.", {
      channelName: "agents",
      replyPubkey: TEST_IDENTITIES.bob.pubkey,
    });
    await nameOpenThread(page, "Agent thread");

    await page.getByTestId("open-threads-view").click();
    await page.getByTestId("threads-view-row").click();
    const panel = page.getByTestId("message-thread-panel");
    await expect(panel).toContainText("Root of a thread with an agent.");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as MockMessageWindow
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "agents",
              kind: 20002,
            }) ?? false,
        ),
      )
      .toBe(true);

    const emitTyping = (pubkey: string) =>
      page.evaluate(
        ({ pubkey, threadHeadId }) => {
          (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
            channelName: "agents",
            pubkey,
            threadHeadId,
          });
        },
        { pubkey, threadHeadId: rootId },
      );
    // Alice is an agent in #agents; Charlie is not.
    await emitTyping(TEST_IDENTITIES.alice.pubkey);
    await expect(
      panel.getByTestId("bot-activity-composer-trigger"),
    ).toBeVisible();
    await emitTyping(TEST_IDENTITIES.charlie.pubkey);
    await expect(panel.getByTestId("message-typing-indicator")).toBeVisible();
  });

  test("View activity log from the Threads view pane opens the thread's channel", async ({
    page,
  }) => {
    const agentPubkey =
      "a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00";
    // #general comes first in the agent's channels, so the route fallback
    // would land there instead of the thread's channel.
    await installMockBridge(page, {
      relayAgents: [
        {
          pubkey: agentPubkey,
          name: "nadia",
          channelNames: ["general", "agents"],
          respondTo: "anyone",
        },
      ],
    });
    await page.goto("/");
    await openThread(page, "Root of a thread nadia replied to.", {
      channelName: "agents",
      replyPubkey: agentPubkey,
    });
    await nameOpenThread(page, "Nadia thread");

    await page.getByTestId("open-threads-view").click();
    await page.getByTestId("threads-view-row").click();
    const panel = page.getByTestId("message-thread-panel");
    const reply = panel
      .getByTestId("message-row")
      .filter({ hasText: "First reply." });
    await reply.getByTestId("message-author").hover();
    const viewActivity = page.getByTestId(
      `user-profile-view-activity-${agentPubkey}`,
    );
    await viewActivity.click();

    await expect(page.getByTestId("chat-title")).toHaveText("agents");
    await expect(page).toHaveURL(
      new RegExp(
        `/channels/94a444a4-c0a3-5966-ab05-530c6ddc2301\\?.*agentSession=${agentPubkey}`,
      ),
    );
  });

  test("a Threads row shows a live timer while an agent works in that thread", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    const workedRootId = await openThread(page, "Thread an agent works in.", {
      channelName: "agents",
    });
    await nameOpenThread(page, "Worked thread");
    await openThread(page, "Thread nobody works in.", {
      channelName: "agents",
    });
    await nameOpenThread(page, "Idle thread");
    const channelId = await page
      .getByTestId("channel-agents")
      .getAttribute("data-channel-id");
    if (!channelId) throw new Error("#agents is missing its channel id.");

    await page.getByTestId("open-threads-view").click();
    const workedRow = page
      .getByTestId("threads-view-row")
      .filter({ hasText: "Worked thread" });
    const idleRow = page
      .getByTestId("threads-view-row")
      .filter({ hasText: "Idle thread" });
    await expect(workedRow).toBeVisible();
    await expect(idleRow).toBeVisible();

    const seedTurn = (
      kind: "turn_started" | "turn_completed",
      threadRootEventId?: string,
    ) =>
      page.evaluate(
        (input) => {
          (window as MockMessageWindow).__BUZZ_E2E_SEED_ACTIVE_TURNS__?.(input);
        },
        {
          agentPubkey: TEST_IDENTITIES.alice.pubkey,
          channelId,
          kind,
          threadRootEventId,
          turnId: "thread-row-timer",
        },
      );

    const sidebarBadge = page.getByTestId("threads-working");

    // A channel-scoped turn has no thread, so no row lights up.
    await seedTurn("turn_started");
    await expect(page.getByTestId("threads-view-row-working")).toHaveCount(0);
    await expect(sidebarBadge).toHaveCount(0);
    await seedTurn("turn_completed");

    // An untitled thread isn't listed, so the sidebar entry stays dark too.
    await seedTurn("turn_started", "c".repeat(64));
    await expect(page.getByTestId("threads-view-row-working")).toHaveCount(0);
    await expect(sidebarBadge).toHaveCount(0);
    await seedTurn("turn_completed", "c".repeat(64));

    await seedTurn("turn_started", workedRootId);
    await expect(workedRow.getByTestId("threads-view-row-working")).toHaveText(
      /^\d+s$/,
    );
    await expect(idleRow.getByTestId("threads-view-row-working")).toHaveCount(
      0,
    );
    await expect(sidebarBadge).toHaveText(/^\d+s$/);
    await expect(sidebarBadge).toHaveAttribute("title", "1 thread working");

    await seedTurn("turn_completed", workedRootId);
    await expect(page.getByTestId("threads-view-row-working")).toHaveCount(0);
    await expect(sidebarBadge).toHaveCount(0);
  });

  test("the selected Threads item shows its unread dot while a thread is open", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      localStorage.setItem("buzz.channels.threadViewMode", "focus");
      // A theme whose selected row is filled with the blue accent, where a
      // primary-colored dot disappears.
      localStorage.setItem("buzz-theme", "catppuccin-latte");
      localStorage.setItem("buzz-accent-color", "#3b82f6");
    });
    await installMockBridge(page);
    await page.goto("/");
    const otherRoot = await openThread(page, "Root of the other thread.");
    await nameOpenThread(page, "Other thread");
    await openThread(page, "Root of the thread being read.", {
      replyCreatedAt: Math.floor(Date.now() / 1000) + 60,
    });
    await nameOpenThread(page, "Thread being read");
    await page.keyboard.press("Escape");

    await page.getByTestId("open-threads-view").click();
    await page
      .getByTestId("threads-view-row")
      .filter({ hasText: "Thread being read" })
      .click();
    await expect(page.getByTestId("focus-thread-drawer")).toBeVisible();

    // The drawer covers the other row's "1 new", so the sidebar has to say it.
    await emitReply(page, otherRoot, "News in the other thread.");
    const dot = page.getByTestId("threads-unread-dot");
    await expect(dot).toBeVisible();
    // Drawn in the selected row's text color, so it shows on any row fill
    // (a primary dot vanished on the primary-filled selection).
    const colors = await dot.evaluate((element) => {
      const button = element.closest("button") as HTMLElement;
      return {
        fill: getComputedStyle(button).backgroundColor,
        text: getComputedStyle(button).color,
        dot: getComputedStyle(element).backgroundColor,
      };
    });
    expect(colors.dot).toBe(colors.text);
    expect(colors.dot).not.toBe(colors.fill);
  });

  test("a pinned thread stays above more recently active ones", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await openThread(page, "Root of the thread to pin.");
    await nameOpenThread(page, "Pin me");
    // Both threads are created within the same second; without a later reply
    // their activity ties and the "most recent first" order is arbitrary.
    const busyRoot = await openThread(page, "Root of a busier thread.", {
      replyCreatedAt: Math.floor(Date.now() / 1000) + 60,
    });
    await nameOpenThread(page, "Busy thread");

    await page.getByTestId("open-threads-view").click();
    const rows = page.getByTestId("threads-view-row");
    await expect(rows).toHaveText([/Busy thread/, /Pin me/]);
    await expect(page.getByTestId("threads-view-pinned")).toHaveCount(0);

    const pinMe = page
      .getByRole("listitem")
      .filter({ hasText: "Pin me" })
      .getByTestId("threads-view-row-pin");
    await pinMe.click();
    await expect(pinMe).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("threads-view-pinned")).toContainText(
      "Pin me",
    );
    await expect(rows).toHaveText([/Pin me/, /Busy thread/]);

    // New activity elsewhere doesn't move a thread above a pin, and the pin
    // survives leaving and reopening the view.
    await emitReply(page, busyRoot, "More activity in the busy thread.");
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
    await page.getByTestId("open-threads-view").click();
    await expect(rows).toHaveText([/Pin me/, /Busy thread/]);

    await pinMe.click();
    await expect(page.getByTestId("threads-view-pinned")).toHaveCount(0);
    await expect(pinMe).toHaveAttribute("aria-pressed", "false");
    await expect(rows).toHaveText([/Busy thread/, /Pin me/]);
  });

  test("a title another user sets appears before anyone replies", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
    await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL_NAME);
    await expect
      .poll(() =>
        page.evaluate(
          (name) =>
            (
              window as MockMessageWindow
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: name,
              kind: 45010,
            }) ?? false,
          CHANNEL_NAME,
        ),
      )
      .toBe(true);

    // Visit Threads first so the (empty) title list is cached and fresh.
    await page.getByTestId("open-threads-view").click();
    await expect(page.getByTestId("threads-view-row")).toHaveCount(0);
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();

    const rootId = await page.evaluate(
      ({ channelName, pubkey }) =>
        (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: "Question for an agent, no reply yet.",
          pubkey,
        })?.id ?? null,
      { channelName: CHANNEL_NAME, pubkey: MOCK_IDENTITY_PUBKEY },
    );
    expect(rootId).not.toBeNull();
    await expect(
      page.getByText("Question for an agent, no reply yet."),
    ).toBeVisible();

    await page.evaluate(
      ({ channelName, rootId }) =>
        (window as MockMessageWindow).__BUZZ_E2E_SET_REMOTE_THREAD_TITLE__?.({
          channelName,
          rootId,
          title: "Agent-named thread",
        }),
      { channelName: CHANNEL_NAME, rootId: rootId as string },
    );

    // Well inside the 30s staleTime: only the live refresh can deliver it.
    await expect(page.getByTestId("thread-title-line")).toHaveText(
      "Agent-named thread",
      { timeout: 5_000 },
    );
    await page.getByTestId("open-threads-view").click();
    await expect(page.getByTestId("threads-view-row")).toHaveText([
      /Agent-named thread/,
    ]);
  });

  test("filter the Threads view by channel", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await setRemoteTitles(page, [
      { channelName: "random", rootId: "c".repeat(64), title: "Random one" },
      { channelName: CHANNEL_NAME, rootId: "a".repeat(64), title: "Eng one" },
      { channelName: CHANNEL_NAME, rootId: "b".repeat(64), title: "Eng two" },
    ]);

    await page.getByTestId("open-threads-view").click();
    const rows = page.getByTestId("threads-view-row");
    await expect(rows).toHaveCount(3);
    const filter = page.getByTestId("threads-channel-filter");
    await expect(filter).toHaveText("All channels");

    await filter.click();
    await expect(page.getByTestId("threads-channel-filter-option")).toHaveText([
      /All channels\s*3/,
      /#engineering\s*2/,
      /#random\s*1/,
    ]);
    await page
      .getByTestId("threads-channel-filter-option")
      .filter({ hasText: "#engineering" })
      .click();
    await expect(filter).toHaveText(`#${CHANNEL_NAME}`);
    await expect(rows).toHaveCount(2);
    await expect(rows.filter({ hasText: "Random one" })).toHaveCount(0);

    // The filter holds across leaving and coming back.
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
    await page.getByTestId("open-threads-view").click();
    await expect(filter).toHaveText(`#${CHANNEL_NAME}`);
    await expect(rows).toHaveCount(2);

    await filter.click();
    await page
      .getByTestId("threads-channel-filter-option")
      .filter({ hasText: "All channels" })
      .click();
    await expect(rows).toHaveCount(3);
  });

  test("a thread in a channel the user hasn't joined still opens the channel", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await setRemoteTitles(page, [
      { channelName: "design", rootId: "d".repeat(64), title: "Design review" },
      {
        channelName: CHANNEL_NAME,
        rootId: "e".repeat(64),
        title: "Eng thread",
      },
    ]);

    await page.getByTestId("open-threads-view").click();
    await page
      .getByTestId("threads-view-row")
      .filter({ hasText: "Design review" })
      .click();
    await expect(page.getByTestId("chat-title")).toHaveText("design");
  });

  test("Threads follows the Focus thread layout and can switch to beside", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      localStorage.setItem("buzz.channels.threadViewMode", "focus");
    });
    await installMockBridge(page);
    await page.goto("/");
    await openThread(page, "Root message for the focus drawer.");
    await nameOpenThread(page, "Focus drawer thread");
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("focus-thread-drawer")).toHaveCount(0);

    await page.getByTestId("open-threads-view").click();
    const row = page.getByTestId("threads-view-row");
    await row.click();

    // Focus opens the large drawer over the list, which goes inert.
    const drawer = page.getByTestId("focus-thread-drawer");
    await expect(drawer.getByTestId("message-thread-open-channel")).toHaveText(
      `Focus drawer thread · #${CHANNEL_NAME}`,
    );
    await expect(page.getByTestId("threads-view")).toBeVisible();
    await expect(
      page.locator("[inert]").getByTestId("threads-view"),
    ).toHaveCount(1);

    // The scrim goes back to the list.
    await page
      .getByRole("button", { name: "Back to Threads" })
      .click({ position: { x: 20, y: 300 } });
    await expect(drawer).toHaveCount(0);
    await expect(
      page.locator("[inert]").getByTestId("threads-view"),
    ).toHaveCount(0);

    // The header toggle switches to the side pane and saves the choice.
    await row.click();
    await drawer.getByTestId("thread-view-mode-toggle").click();
    await expect(drawer).toHaveCount(0);
    const sidePanel = page.getByTestId("message-thread-panel");
    await expect(
      sidePanel.getByTestId("message-thread-open-channel"),
    ).toHaveText(`Focus drawer thread · #${CHANNEL_NAME}`);
    await expect(
      page.locator("[inert]").getByTestId("threads-view"),
    ).toHaveCount(0);
    expect(
      await page.evaluate(() =>
        localStorage.getItem("buzz.channels.threadViewMode"),
      ),
    ).toBe("split");

    // And back to Focus from the side pane.
    await sidePanel.getByTestId("thread-view-mode-toggle").click();
    await expect(drawer.getByTestId("message-thread-open-channel")).toHaveText(
      `Focus drawer thread · #${CHANNEL_NAME}`,
    );
  });

  test("clicking Threads in the sidebar closes the open thread", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      localStorage.setItem("buzz.channels.threadViewMode", "focus");
    });
    await installMockBridge(page);
    await page.goto("/");
    await openThread(page, "Root message for the sidebar reset.");
    await nameOpenThread(page, "Sidebar reset thread");
    await page.keyboard.press("Escape");

    await page.getByTestId("open-threads-view").click();
    const row = page.getByTestId("threads-view-row");
    const drawer = page.getByTestId("focus-thread-drawer");

    // Focus: the drawer covers the list, but not the sidebar.
    await row.click();
    await expect(drawer.getByTestId("message-thread-open-channel")).toHaveText(
      `Sidebar reset thread · #${CHANNEL_NAME}`,
    );
    await page.getByTestId("open-threads-view").click();
    await expect(drawer).toHaveCount(0);
    await expect(
      page.locator("[inert]").getByTestId("threads-view"),
    ).toHaveCount(0);

    // Beside: the side pane closes the same way.
    await row.click();
    await drawer.getByTestId("thread-view-mode-toggle").click();
    const sidePanel = page.getByTestId("message-thread-panel");
    await expect(
      sidePanel.getByTestId("message-thread-open-channel"),
    ).toHaveText(`Sidebar reset thread · #${CHANNEL_NAME}`);
    await page.getByTestId("open-threads-view").click();
    await expect(sidePanel).toHaveCount(0);
    await expect(row).not.toHaveAttribute("data-selected", "true");

    // A reopen still works after a reset.
    await row.click();
    await expect(
      sidePanel.getByTestId("message-thread-open-channel"),
    ).toHaveText(`Sidebar reset thread · #${CHANNEL_NAME}`);
  });
});
