import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

test("first message in a new chat is sent and rendered", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => {
    consoleErrors.push(`pageerror: ${error.message}`);
  });

  await installMockBridge(page);
  await page.goto("/#/chats");

  const composer = page.locator("[contenteditable='true'], textarea").first();
  await expect(composer).toBeVisible();
  await composer.click();
  await composer.fill("Hello Fizz, first message");
  await composer.press("Enter");

  // Give the create+send flow time to run.
  await page.waitForTimeout(4_000);

  console.log("URL after send:", page.url());
  console.log("Console errors:", JSON.stringify(consoleErrors, null, 2));

  await page.screenshot({
    path: "test-results/chats-first-message.png",
    fullPage: false,
  });

  // The chat should have been created and navigated to...
  await expect(page).toHaveURL(/\/chats\/.+/);
  // ...and the first message should be visible in the conversation (the same
  // text also appears in the sidebar item and chat title, so scope the
  // assertion to the message list).
  await expect(
    page.getByLabel("Chat messages").getByText("Hello Fizz, first message"),
  ).toBeVisible();

  // Just-sent rows slide in via the entrance animation (recency-gated, so
  // only fresh messages animate — not history on chat open).
  const animatedRow = page
    .getByLabel("Chat messages")
    .locator(".buzz-message-entrance", { hasText: "Hello Fizz, first message" })
    .first();
  await expect(animatedRow).toBeVisible();
  expect(
    await animatedRow.evaluate(
      (element) => window.getComputedStyle(element).animationName,
    ),
  ).toBe("buzz-message-entrance");

  // Once the conversation develops (two agent replies), the chat auto-titles
  // itself with a succinct subject line ("Hello Fizz, first message" →
  // "First message"), rendered through the animated title swap.
  await expect
    .poll(async () =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: "Hello Fizz, first message" },
      ),
    )
    .toBe(true);

  const fizzPubkey = await page.evaluate(async () => {
    const win = window as Window & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
        command: string,
        payload: unknown,
      ) => Promise<unknown>;
    };
    const agents = (await win.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "list_managed_agents",
      {},
    )) as Array<{ name: string; pubkey: string }> | undefined;
    return agents?.find((agent) => agent.name === "Fizz")?.pubkey ?? null;
  });
  expect(fizzPubkey).toBeTruthy();

  await page.evaluate(
    ({ pubkey }) => {
      const win = window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          createdAt?: number;
          mentionPubkeys?: string[];
          pubkey?: string;
        }) => unknown;
      };
      // Explicit ascending timestamps: same-second events sort unstably and
      // can flip which agent reply the run-collapse keeps visible.
      const base = Math.floor(Date.now() / 1000);
      win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Hello Fizz, first message",
        content: "Sure — that first message says hello.",
        createdAt: base,
        pubkey: pubkey ?? undefined,
      });
      win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Hello Fizz, first message",
        content:
          "Done! I've opened https://github.com/block/buzz/pull/1460 with the changes.",
        createdAt: base + 2,
        pubkey: pubkey ?? undefined,
      });
      // A narration-styled FINAL reply must still render: persisted
      // messages are never filtered by the transcript's narration
      // heuristics (this exact phrasing used to be silently dropped).
      win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Hello Fizz, first message",
        content: "Done! I've sent the summary with details to the channel.",
        createdAt: base + 3,
        pubkey: pubkey ?? undefined,
      });
      // A human message with a mention tag: @bob must render as a chip
      // (alice's pubkey is a mock profile fixture; bob's resolves via the
      // message's p tag).
      win.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Hello Fizz, first message",
        content: "Loop in @bob for the review.",
        createdAt: base + 4,
        mentionPubkeys: [
          "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
        ],
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
      });
    },
    { pubkey: fizzPubkey },
  );

  await expect(
    page
      .getByLabel("Chat messages")
      .getByText("Done! I've sent the summary with details to the channel."),
  ).toBeVisible({ timeout: 10_000 });

  // Mentions in chat messages render as chips, same as channels.
  await expect(
    page.getByLabel("Chat messages").locator("[data-mention]", {
      hasText: "bob",
    }),
  ).toBeVisible({ timeout: 10_000 });

  await expect(
    page.getByRole("heading", { name: "First message" }),
  ).toBeVisible({ timeout: 10_000 });
  const animatedTitle = page
    .getByTestId("chat-title")
    .getByTestId("animated-title-text");
  await expect(animatedTitle).toBeVisible();
  // The measured width must fit the whole title — a collapsed box renders
  // "F…" instead of the text.
  await expect
    .poll(async () =>
      animatedTitle.evaluate((element) => {
        const sizer = element.querySelector<HTMLElement>(
          "span[aria-hidden='true']",
        );
        if (!sizer) return -1;
        return element.getBoundingClientRect().width - sizer.scrollWidth;
      }),
    )
    .toBeGreaterThanOrEqual(-1);

  // Agent-authored PR links render the prominent agent-work card inline in
  // the message; the top-right work panel shows the standard rich card plus
  // the PR's source branch.
  await expect(
    page.locator("[data-link-preview='github-pull-request-agent']"),
  ).toHaveCount(1, { timeout: 10_000 });
  // The drawer is purely a remembered preference (default closed) — open
  // it explicitly; the choice persists across chats and restarts.
  const workPanel = page.getByTestId("chat-work-panel");
  await expect(workPanel).not.toBeVisible();
  await page.getByTestId("toggle-work-panel").click();
  await expect(workPanel).toBeVisible();
  await expect(workPanel).toContainText("kennylopez-chatmode");
  await expect(
    workPanel.locator("[data-link-preview='github-pull-request']"),
  ).toBeVisible();

  // CI monitor summary shows check state plus unreplied review threads.
  const ciMonitor = page.getByTestId("chat-ci-monitor");
  await expect(ciMonitor).toContainText("CI passing");
  await expect(ciMonitor).toContainText("2 open comments");
  // Collapsed by default; expanding reveals the runs + automation toggles.
  await expect(page.getByTestId("automation-auto-fix-ci")).not.toBeVisible();
  await ciMonitor.locator("summary").click();
  await expect(ciMonitor).toContainText("ci / unit-tests");
  await expect(page.getByTestId("automation-auto-fix-ci")).toBeVisible();
  await expect(page.getByTestId("automation-address-comments")).toBeVisible();

  // Arming address-comments fires the automation prompt (2 open threads in
  // the mock). The message goes out tagged — but no bubble renders: the
  // automation stays invisible in the timeline.
  await page.getByTestId("automation-address-comments").click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
                command: string;
                payload?: { content?: string; clientTags?: string[][] };
              }>;
            }
          ).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []
        ).some(
          (entry) =>
            entry.command === "send_channel_message" &&
            entry.payload?.content?.includes("unanswered review comments") &&
            entry.payload?.clientTags?.some(
              (tag) => tag[0] === "client" && tag[1] === "automation",
            ),
        ),
      ),
    )
    .toBe(true);
  // The instruction renders as a collapsed marker row, not a bubble.
  const automationRow = page.getByTestId("chat-automation-row");
  await expect(automationRow).toBeVisible({ timeout: 10_000 });
  await expect(automationRow).toContainText(
    "Asked Fizz to address the review comments",
  );
  await expect(
    page.getByLabel("Chat messages").getByText("unanswered review comments"),
  ).not.toBeVisible();

  // Manual overrides: open comments expose "Run now"; green CI does not.
  await expect(page.getByTestId("automation-run-comments-now")).toBeVisible();
  await expect(page.getByTestId("automation-run-ci-now")).toHaveCount(0);

  // The header's PR button toggles the panel.
  await page.getByTestId("toggle-work-panel").click();
  await expect(workPanel).not.toBeVisible();
  await page.getByTestId("toggle-work-panel").click();
  await expect(workPanel).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: "test-results/agent-pr-card.png" });
});

test("new chat screen shows agent, directory, and invite preset cards", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/#/chats");

  const agentCard = page.getByTestId("chat-preset-agent");
  const directoryCard = page.getByTestId("chat-preset-directory");
  const inviteCard = page.getByTestId("chat-preset-invite");
  await expect(agentCard).toBeVisible();
  await expect(agentCard).toContainText("Fizz");
  await expect(directoryCard).toBeVisible();
  await expect(inviteCard).toBeVisible();
  await expect(inviteCard).toContainText("Invite");

  // Agent picker lists the default agent option.
  await agentCard.click();
  await expect(
    page.getByRole("dialog").getByText("Default agent"),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  // Invite picker searches the user directory and stores a selection.
  await inviteCard.click();
  await page.getByPlaceholder("Search people").fill("alice");
  await page.getByRole("dialog").getByText("alice").click();
  await page.keyboard.press("Escape");
  await expect(inviteCard).toContainText("1 invited");

  await page.screenshot({ path: "test-results/chat-start-presets.png" });
});

test("a PR can be pinned to the work panel manually", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/#/chats");
  const composer = page.locator("[contenteditable='true'], textarea").first();
  await expect(composer).toBeVisible();
  await composer.click();
  await composer.fill("pin a pr here");
  await composer.press("Enter");
  await expect(page).toHaveURL(/\/chats\/.+/);

  await page.getByTestId("toggle-work-panel").click();
  const workPanel = page.getByTestId("chat-work-panel");
  await expect(workPanel).toBeVisible();

  await page.getByTestId("chat-work-pin-pr").click();
  await page
    .getByTestId("chat-work-pin-input")
    .fill("https://github.com/block/buzz/pull/1460");
  await page.getByRole("button", { name: "Pin", exact: true }).click();
  await expect(
    workPanel.locator("[data-link-preview='github-pull-request']"),
  ).toBeVisible({ timeout: 10_000 });

  // Unlink clears it back to the empty state.
  await page.getByTestId("chat-work-unlink-pr").click();
  await expect(
    workPanel.locator("[data-link-preview='github-pull-request']"),
  ).toHaveCount(0);
  await expect(page.getByTestId("chat-work-pin-pr")).toBeVisible();
});

test("sidebar chat title shimmers while the agent has an active turn", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/#/chats");

  const composer = page.locator("[contenteditable='true'], textarea").first();
  await expect(composer).toBeVisible();
  await composer.click();
  await composer.fill("Spin up a new worktree to look at the panel");
  await composer.press("Enter");
  await expect(page).toHaveURL(/\/chats\/.+/);
  const chatId = decodeURIComponent(
    new URL(page.url().replace("/#/", "/")).pathname.split("/").at(-1) ?? "",
  );
  expect(chatId.length).toBeGreaterThan(0);

  // Quiet agent: no shimmer on the sidebar title.
  const shimmerTitle = page.locator(
    "[data-shimmer-text*='Spin up a new worktree']",
  );
  await expect(shimmerTitle).toHaveCount(0);

  // While no turn is active the composer offers plain send.
  await expect(page.getByTestId("send-message")).toBeVisible();
  await expect(page.getByTestId("stop-agent")).toHaveCount(0);

  // Seed an active turn for this chat's agent — the row title must pick up
  // the accent shimmer (class + animated overlay), same store the working
  // row reads, and the composer's send button must become a stop button.
  await page.evaluate(
    async ({ channelId }) => {
      const win = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: unknown,
        ) => Promise<unknown>;
        __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
          agentPubkey: string;
          channelId: string;
          turnId: string;
        }) => void;
      };
      const agents = (await win.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
        "list_managed_agents",
      )) as Array<{ pubkey: string }>;
      win.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey: agents[0]?.pubkey ?? "ab".repeat(32),
        channelId,
        turnId: "turn-shimmer-1",
      });
    },
    { channelId: chatId },
  );

  await expect(shimmerTitle).toHaveCount(1);
  await expect(shimmerTitle).toHaveClass(/buzz-shimmer/);
  await expect(shimmerTitle).toHaveClass(/buzz-shimmer-accent/);
  expect(
    await shimmerTitle.evaluate(
      (element) => window.getComputedStyle(element, "::before").animationName,
    ),
  ).toBe("buzz-shimmer");

  // Send became stop; clicking it sends the cancel-turn control frame.
  const stopButton = page.getByTestId("stop-agent");
  await expect(stopButton).toBeVisible();
  await expect(page.getByTestId("send-message")).toHaveCount(0);
  await stopButton.click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? []
        ).includes("build_observer_control_event"),
      ),
    )
    .toBe(true);
});
