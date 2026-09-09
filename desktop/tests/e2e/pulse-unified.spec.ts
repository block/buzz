import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const agentKey = "c1".repeat(32);

async function seed(
  page: Page,
  agentNeedsInput = false,
  expandedBriefing = false,
) {
  await page.route("**/__pulse/briefing", async (route) => {
    const body = route.request().postDataJSON();
    const source = body.conversations.find(
      (item: { messages: { body: string }[] }) =>
        item.messages.some((m) =>
          m.body.includes(
            agentNeedsInput ? "blocked on deployment" : "Got a few minutes",
          ),
        ),
    );
    await route.fulfill({
      json: {
        highlights: expandedBriefing
          ? body.conversations
              .slice(0, 5)
              .map((item: { id: string }, i: number) => ({
                summary: `Activity highlight ${i + 1}`,
                conversationIds: [item.id],
              }))
          : source
            ? [
                {
                  summary: agentNeedsInput
                    ? "Scout needs approval for deployment access"
                    : "Alice wants to review the Pulse prototype",
                  conversationIds: [source.id],
                },
              ]
            : [],
      },
    });
  });
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [
      {
        pubkey: agentKey,
        name: "Scout",
        channelNames: ["engineering"],
        status: "online",
      },
    ],
    searchProfiles: [
      { pubkey: agentKey, displayName: "Scout", isAgent: true },
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        displayName: "Alice",
        isAgent: false,
      },
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        displayName: "Bob",
        isAgent: false,
      },
    ],
  });
  await page.goto("/");
  await expect(page.getByTestId("open-pulse-view")).toBeVisible();
  for (const name of ["engineering", "random", "alice-tyler"]) {
    await page.getByTestId(`channel-${name}`).click();
    await page.waitForFunction(
      (channelName) =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }),
      name,
    );
    await page.evaluate(
      async ({
        name,
        alice,
        bob,
        agentKey,
        agentNeedsInput,
        expandedBriefing,
      }) => {
        const { pubkey: me } = await window.__TAURI_INTERNALS__.invoke<{
          pubkey: string;
        }>("get_identity");
        const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        if (!emit) throw new Error("Mock bridge missing");
        const now = Math.floor(Date.now() / 1000);
        if (name === "engineering") {
          emit({
            channelName: name,
            content: agentNeedsInput
              ? "I'm blocked on deployment access. I need your approval to continue."
              : "Finished reviewing the relay changes. All checks passed, and the branch is ready for a human review.\n\nThe only follow-up: decide whether we want the new timeout as the default.",
            mentionPubkeys: agentNeedsInput ? [me] : [],
            pubkey: agentKey,
            createdAt: now - 300,
          });
        } else if (name === "random") {
          if (expandedBriefing) {
            for (const [index, content] of [
              "The design review is ready for feedback",
              "The release plan needs a final approval",
            ].entries())
              emit({
                channelName: name,
                content,
                pubkey: bob,
                createdAt: now - 180 - index,
              });
          }
          const root = emit({
            channelName: name,
            content:
              "What if catching up felt more like reading a conversation?\n\nTrying a single feed for channels, DMs, and the agents working alongside us. Everything stays in its original place.",
            pubkey: alice,
            createdAt: now - 120,
          });
          emit({
            channelName: name,
            content:
              "Yes to this. Keep the audience visible so I always know where my reply is going.",
            pubkey: bob,
            parentEventId: root.id,
            createdAt: now - 90,
          });
        } else {
          emit({
            channelName: name,
            content:
              "Got a few minutes to look at the Pulse prototype together? I have some thoughts on the private conversation cards.",
            pubkey: alice,
            mentionPubkeys: [me],
            createdAt: now - 30,
          });
        }
      },
      {
        name,
        alice: TEST_IDENTITIES.alice.pubkey,
        bob: TEST_IDENTITIES.bob.pubkey,
        agentKey,
        agentNeedsInput,
        expandedBriefing,
      },
    );
  }
  await page.getByTestId("open-pulse-view").click();
  await expect(page.getByTestId("pulse-briefing")).toBeVisible();
  await expect(page.getByTestId("open-pulse-view")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Toggle Sidebar", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
  await expect(
    page.getByRole("searchbox", { name: "Search loaded feed" }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
          getQueryCache: () => {
            getAll: () => Array<{
              queryKey: unknown[];
              state: { status: string; error?: Error };
            }>;
          };
        };
        return client
          .getQueryCache()
          .getAll()
          .filter((q) =>
            ["pulse-unified", "contact-list"].includes(String(q.queryKey[0])),
          )
          .map((q) => ({
            key: q.queryKey[0],
            status: q.state.status,
            error: q.state.error?.message,
          }));
      }),
    )
    .toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: "pulse-unified", status: "success" }),
      ]),
    );
}

test.use({ viewport: { width: 1440, height: 1000 } });

test("briefing shows intent summaries linked to their source conversations", async ({
  page,
}) => {
  await seed(page);
  const briefing = page.getByTestId("pulse-briefing");
  const highlight = briefing.getByTestId("pulse-briefing-highlight");
  await expect(highlight).toContainText(
    "Alice wants to review the Pulse prototype",
  );
  await expect(highlight).not.toContainText("Got a few minutes");
  await expect(
    highlight.getByRole("button", { name: "Open Alice's profile" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("unified-pulse").screenshot({
    path: "test-results/pulse-prototype/10-visual-briefing.png",
  });
  await highlight.getByRole("button", { name: "Reply", exact: true }).click();
  await expect(
    highlight.getByText("Reply privately in alice-tyler", { exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke.bind(internals);
    (window as unknown as { summarySends: unknown[] }).summarySends = [];
    internals.invoke = async (command, args, options) => {
      if (command === "send_channel_message")
        (window as unknown as { summarySends: unknown[] }).summarySends.push(
          args,
        );
      return invoke(command, args, options);
    };
  });
  await highlight
    .locator('[contenteditable="true"]')
    .pressSequentially("Yes, let's review it together.");
  await highlight
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { summarySends: unknown[] }).summarySends
            .length,
      ),
    )
    .toBe(1);
  const send = await page.evaluate(
    () =>
      (
        window as unknown as {
          summarySends: { channelId: string; parentEventId: string }[];
        }
      ).summarySends[0],
  );
  await expect(highlight).toHaveAttribute(
    "data-conversation-id",
    `${send.channelId}:${send.parentEventId}`,
  );
  await highlight
    .getByRole("button", { name: "Open conversation", exact: true })
    .click();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(1);
  await expect(page.getByTestId("pulse-conversation")).toContainText(
    "Got a few minutes",
  );
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/08-content-recap.png" });
});

test("For you briefing prioritizes agent requests and focuses matching conversations", async ({
  page,
}) => {
  await seed(page, true);
  const briefing = page.getByTestId("pulse-briefing");
  const highlight = briefing.getByTestId("pulse-briefing-highlight");
  await expect(highlight).toContainText(
    "Scout needs approval for deployment access",
  );
  const request = highlight.getByRole("button", {
    name: "Open conversation",
    exact: true,
  });
  await expect(request).toBeVisible();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
  await expect
    .poll(
      async () =>
        (await page.getByTestId("pulse-main-container").boundingBox())?.width,
    )
    .toBe(800);
  await expect(page.getByTestId("pulse-main-container")).toHaveCSS(
    "border-radius",
    "24px",
  );
  await expect(page.getByTestId("unified-pulse")).toHaveCSS(
    "padding-top",
    "12px",
  );
  await expect(page.getByTestId("unified-pulse")).toHaveCSS(
    "padding-bottom",
    "12px",
  );
  await expect(page.getByTestId("unified-pulse")).toHaveCSS(
    "background-color",
    "rgb(247, 247, 248)",
  );
  await request.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("button", { name: "Search", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(1);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/07-briefing.png" });
  await page.getByRole("button", { name: "Show all activity" }).click();
  await expect(
    page
      .getByTestId("pulse-conversation")
      .filter({ hasText: "Got a few minutes" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "DMs", exact: true }).click();
  await expect(briefing).toHaveCount(0);
});

test("mixed feed, type filters, search, and DM reply destination", async ({
  page,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(
    page
      .getByTestId("pulse-conversation")
      .filter({ hasText: "What if catching up" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/01-feed.png" });
  const dm = page
    .getByTestId("pulse-conversation")
    .filter({ hasText: "Got a few minutes" });
  await expect(
    dm.getByRole("button", { name: "DM · alice-tyler", exact: true }),
  ).toBeVisible();
  await dm.getByRole("button", { name: /^Reply/ }).click();
  await expect(
    dm.getByText("Reply privately in alice-tyler", { exact: true }),
  ).toBeVisible();
  await waitForAnimations(page);
  await dm.screenshot({
    path: "test-results/pulse-prototype/02-private-reply.png",
  });
  await page.evaluate(() => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke.bind(internals);
    (window as unknown as { pulseSends: unknown[] }).pulseSends = [];
    internals.invoke = async (command, args, options) => {
      if (command === "send_channel_message")
        (window as unknown as { pulseSends: unknown[] }).pulseSends.push(args);
      return invoke(command, args, options);
    };
  });
  await dm
    .locator('[contenteditable="true"]')
    .pressSequentially("Absolutely — let's walk through it.");
  await dm.getByRole("button", { name: "Send message", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { pulseSends: unknown[] }).pulseSends.length,
      ),
    )
    .toBe(1);
  const send = await page.evaluate(
    () =>
      (
        window as unknown as {
          pulseSends: Array<{
            channelId: string;
            parentEventId: string;
            content: string;
          }>;
        }
      ).pulseSends[0],
  );
  await expect(dm).toHaveAttribute(
    "data-conversation-id",
    `${send.channelId}:${send.parentEventId}`,
  );
  await dm.getByRole("button", { name: /^View \d+ repl/ }).click();
  await expect(dm).toContainText("Absolutely — let's walk through it.");
  await page
    .getByTestId("unified-pulse")
    .getByRole("button", { name: "Agents", exact: true })
    .click();
  await expect(
    page
      .getByTestId("pulse-conversation")
      .filter({ hasText: "All checks passed" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page
    .getByRole("searchbox", { name: "Search loaded feed" })
    .fill("timeout");
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(1);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/03-filtered.png" });
});

test("DM split view switches conversations, preserves drafts, and sends to the selected DM", async ({
  page,
}) => {
  await seed(page);
  await page.getByRole("button", { name: "DMs", exact: true }).click();
  const list = page.getByTestId("pulse-dm-list");
  await expect(list).toHaveCSS("width", "200px");
  const detail = page.getByTestId("pulse-dm-detail");
  const alice = list.locator('[data-channel-name="alice-tyler"]');
  const bob = list.locator('[data-channel-name="bob-tyler"]');
  await expect(alice).toHaveCount(1);
  await expect(bob).toHaveCount(1);
  await expect(alice).toContainText("Alice");
  await expect(list).not.toContainText("Got a few minutes");
  await alice.click();
  await expect(alice).toHaveAttribute("aria-current", "true");
  await expect(detail).toContainText("Got a few minutes");
  await expect(detail.getByTestId("chat-title")).toContainText(/alice/i);
  const aliceId = await detail.getAttribute("data-channel-id");
  const input = detail.getByTestId("message-input");
  await input.fill("Draft for Alice only");
  await bob.click();
  await expect(bob).toHaveAttribute("aria-current", "true");
  await expect(detail.getByTestId("chat-title")).toContainText(/bob/i);
  await expect(detail).not.toContainText("Got a few minutes");
  await expect(input).toHaveText("");
  await alice.click();
  await expect(input).toContainText("Draft for Alice only");
  await page.evaluate(() => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke.bind(internals);
    (window as unknown as { dmSends: unknown[] }).dmSends = [];
    internals.invoke = async (command, args, options) => {
      if (command === "sign_event" && args?.kind === 9)
        (window as unknown as { dmSends: unknown[] }).dmSends.push(args);
      return invoke(command, args, options);
    };
  });
  await input.fill("A message from the split DM view");
  await detail
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              dmSends: { tags: string[][]; content: string }[];
            }
          ).dmSends,
      ),
    )
    .toEqual([
      expect.objectContaining({
        tags: expect.arrayContaining([["h", aliceId]]),
        content: "A message from the split DM view",
      }),
    ]);
  await expect(detail).toContainText("A message from the split DM view");
  await expect(page).toHaveURL(/#\/pulse/);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/04-dm-split.png" });
  await page.getByRole("button", { name: "For you", exact: true }).click();
  await expect(page.getByTestId("pulse-dm-view")).toHaveCount(0);
  await expect(page.getByTestId("pulse-briefing")).toBeVisible();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
});

test("Channels split view starts with all channels and opens individual conversations", async ({
  page,
}) => {
  await seed(page);
  await page
    .getByTestId("unified-pulse")
    .getByRole("button", { name: "Channels", exact: true })
    .click();
  const list = page.getByTestId("pulse-channels-list");
  await expect(list).toHaveCSS("width", "200px");
  const all = list.getByRole("button", { name: "All channels", exact: true });
  const aggregate = page.getByTestId("pulse-all-channels-feed");
  await expect(list.getByRole("button").first()).toHaveText("All channels");
  await expect(all).toHaveAttribute("aria-current", "true");
  await expect(
    aggregate
      .getByTestId("pulse-conversation")
      .filter({ hasText: "What if catching up" }),
  ).toBeVisible();
  await expect(aggregate).not.toContainText("Got a few minutes");
  await expect(list.locator('[data-channel-name="alice-tyler"]')).toHaveCount(
    0,
  );
  await expect
    .poll(
      async () =>
        (await page.getByTestId("pulse-channels-view").boundingBox())?.width,
    )
    .toBeLessThanOrEqual(800);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/05-all-channels.png" });
  await list.getByRole("button", { name: "engineering", exact: true }).click();
  const detail = page.getByTestId("pulse-channel-detail");
  await expect(detail.getByTestId("chat-title")).toHaveText("engineering");
  await expect(detail).toContainText("Finished reviewing the relay changes");
  await detail.getByTestId("message-input").fill("Draft for engineering");
  await list.getByRole("button", { name: "random", exact: true }).click();
  await expect(detail.getByTestId("chat-title")).toHaveText("random");
  await expect(detail.getByTestId("message-input")).toHaveText("");
  await list.getByRole("button", { name: "engineering", exact: true }).click();
  await expect(detail.getByTestId("message-input")).toContainText(
    "Draft for engineering",
  );
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/06-channel-detail.png" });
  await all.focus();
  await page.keyboard.press("Enter");
  await expect(all).toHaveAttribute("aria-current", "true");
  await expect(detail).toHaveCount(0);
  await expect(aggregate).toBeVisible();
  await list
    .getByRole("button", { name: "announcements", exact: true })
    .click();
  await expect(detail.getByTestId("chat-title")).toHaveText("announcements");
  await expect(page).toHaveURL(/#\/pulse/);
});

test("top tabs stay aligned above scrolling content and Pulse stays sidebar-free after reload", async ({
  page,
}) => {
  await seed(page);
  const segments = page.getByTestId("pulse-tabs");
  await expect(segments.getByRole("button")).toHaveText([
    "Search",
    "For you",
    "DMs",
    "Channels",
    "Agents",
  ]);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/09-summary-only.png" });
  const search = segments.getByRole("button", { name: "Search", exact: true });
  await search.focus();
  await page.keyboard.press("Enter");
  await expect(search).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("pulse-conversation").first()).toBeVisible();
  const before = await segments.boundingBox();
  const scroll = page.getByTestId("pulse-scroll-area");
  await scroll.evaluate((el) => el.scrollTo({ top: el.scrollHeight }));
  await expect
    .poll(() => scroll.evaluate((el) => el.scrollTop))
    .toBeGreaterThan(0);
  await expect
    .poll(async () => (await segments.boundingBox())?.y)
    .toBe(before?.y);
  await expect(search).toBeInViewport();
  await page.reload();
  await expect(page.getByTestId("pulse-tabs")).toBeVisible();
  await expect(page.getByTestId("open-pulse-view")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Toggle Sidebar", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "For you", exact: true }).click();
  await expect(page.getByTestId("pulse-briefing")).toBeVisible();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
});

test("For you renders more than three summary rows", async ({ page }) => {
  await seed(page, false, true);
  const highlights = page.getByTestId("pulse-briefing-highlight");
  await expect(highlights).toHaveCount(5);
  await highlights.last().scrollIntoViewIfNeeded();
  await expect(highlights.last()).toContainText("Activity highlight 5");
  await expect(
    highlights.last().getByRole("button", { name: "Reply", exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("pulse-tabs")).toBeInViewport();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
});

test("app variations combine conversations by recency and preserve selection and drafts", async ({
  page,
}) => {
  await seed(page);
  const menu = page.getByRole("button", {
    name: "App variations",
    exact: true,
  });
  await menu.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("menuitemradio", { name: "Separate feeds", exact: true }),
  ).toHaveAttribute("aria-checked", "true");
  await page
    .getByRole("menuitemradio", { name: "Combined conversations", exact: true })
    .click();
  await expect(page.getByTestId("pulse-tabs").getByRole("button")).toHaveText([
    "Search",
    "For you",
    "Conversations",
    "Agents",
  ]);
  const list = page.getByTestId("pulse-combined-list");
  const detail = page.getByTestId("pulse-combined-detail");
  const all = list.getByRole("button", { name: "All messages", exact: true });
  const aggregate = page.getByTestId("pulse-all-messages-feed");
  await expect(list.getByRole("button").first()).toHaveText("All messages");
  await expect(all).toHaveAttribute("aria-current", "true");
  await expect(
    aggregate
      .getByTestId("pulse-conversation")
      .filter({ hasText: "Got a few minutes" }),
  ).toBeVisible();
  await expect(
    aggregate
      .getByTestId("pulse-conversation")
      .filter({ hasText: "What if catching up" }),
  ).toBeVisible();
  await expect(list).toHaveCSS("width", "200px");
  await expect(page.getByTestId("pulse-main-container")).toHaveCSS(
    "max-width",
    "800px",
  );
  const alice = list.locator('[data-channel-name="alice-tyler"]');
  const engineering = list.locator('[data-channel-name="engineering"]');
  await expect(alice).toBeVisible();
  await expect(engineering).toBeVisible();
  await expect
    .poll(async () => {
      const names = await list
        .getByRole("button")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-channel-name")),
        );
      return (
        names.indexOf("alice-tyler") < names.indexOf("random") &&
        names.indexOf("random") < names.indexOf("engineering")
      );
    })
    .toBe(true);
  await alice.click();
  await expect(detail.getByTestId("chat-title")).toContainText(/alice/i);
  const aliceId = await detail.getAttribute("data-channel-id");
  const input = detail.getByTestId("message-input");
  await input.fill("Combined view draft for Alice");
  await all.focus();
  await page.keyboard.press("Enter");
  await expect(all).toHaveAttribute("aria-current", "true");
  await expect(detail).toHaveCount(0);
  await expect(
    aggregate
      .getByTestId("pulse-conversation")
      .filter({ hasText: "Got a few minutes" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/12-all-messages.png" });
  await alice.click();
  await expect(input).toHaveText("Combined view draft for Alice");
  await engineering.click();
  await expect(detail.getByTestId("chat-title")).toHaveText("engineering");
  await expect(input).toHaveText("");
  await alice.click();
  await expect(input).toHaveText("Combined view draft for Alice");
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "engineering",
    }),
  );
  await page.evaluate(
    ({ pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "engineering",
        pubkey,
        content: "New activity moves this channel to the top",
        createdAt: Math.floor(Date.now() / 1000) + 60,
      });
    },
    { pubkey: agentKey },
  );
  // The shared bridge has an unrelated all-replies fixture dated in 2999.
  // Compare the conversations seeded here to prove that live activity reorders them.
  await expect
    .poll(async () => {
      const names = await list
        .getByRole("button")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-channel-name")),
        );
      return names.indexOf("engineering") < names.indexOf("alice-tyler");
    })
    .toBe(true);
  await expect(detail).toHaveAttribute("data-channel-id", aliceId ?? "missing");
  await expect(input).toHaveText("Combined view draft for Alice");
  await engineering.click();
  await page.evaluate(() => {
    const internals = window.__TAURI_INTERNALS__;
    const invoke = internals.invoke.bind(internals);
    (window as unknown as { combinedSends: unknown[] }).combinedSends = [];
    internals.invoke = async (command, args, options) => {
      if (command === "sign_event" && args?.kind === 9)
        (window as unknown as { combinedSends: unknown[] }).combinedSends.push(
          args,
        );
      return invoke(command, args, options);
    };
  });
  const engineeringId = await detail.getAttribute("data-channel-id");
  await input.fill("A reply from the combined conversation view");
  await detail
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { combinedSends: unknown[] }).combinedSends,
      ),
    )
    .toEqual([
      expect.objectContaining({
        tags: expect.arrayContaining([["h", engineeringId]]),
        content: "A reply from the combined conversation view",
      }),
    ]);
  await expect(detail).toContainText(
    "A reply from the combined conversation view",
  );
  await waitForAnimations(page);
  await page.getByTestId("unified-pulse").screenshot({
    path: "test-results/pulse-prototype/11-combined-conversations.png",
  });
  await menu.click();
  await expect(
    page.getByRole("menuitemradio", {
      name: "Combined conversations",
      exact: true,
    }),
  ).toHaveAttribute("aria-checked", "true");
  await page
    .getByRole("menuitemradio", { name: "Separate feeds", exact: true })
    .click();
  await expect(page.getByTestId("pulse-tabs").getByRole("button")).toHaveText([
    "Search",
    "For you",
    "DMs",
    "Channels",
    "Agents",
  ]);
  await expect(page.getByTestId("pulse-briefing")).toBeVisible();
  await menu.click();
  await page
    .getByRole("menuitemradio", { name: "Combined conversations", exact: true })
    .click();
  await expect(detail).toHaveAttribute(
    "data-channel-id",
    engineeringId ?? "missing",
  );
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Conversations", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(detail).toHaveAttribute(
    "data-channel-id",
    engineeringId ?? "missing",
  );
  await expect(page.getByTestId("open-pulse-view")).toHaveCount(0);
  await list.locator('[data-channel-name="announcements"]').click();
  await expect(detail.getByTestId("chat-title")).toHaveText("announcements");
  await all.click();
  await page.reload();
  await expect(all).toHaveAttribute("aria-current", "true");
  await expect(aggregate).toBeVisible();
  await expect(detail).toHaveCount(0);
});
