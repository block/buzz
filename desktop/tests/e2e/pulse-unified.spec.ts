import { expect, test, type Page, type Locator } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const agentKey = "c1".repeat(32);

async function boundsOf(locator: Locator) {
  const bounds = await locator.boundingBox();
  if (!bounds)
    throw new Error("Expected a visible element for layout measurement");
  return bounds;
}

async function themeColor(page: Page, token: string) {
  return page.evaluate((token) => {
    const probe = document.createElement("span");
    probe.style.color = `hsl(var(${token}))`;
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, token);
}

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
  await expect(page.locator("[data-message-bubble]")).toHaveCount(0);
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
    .toBe(960);
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
    "rgba(0, 0, 0, 0)",
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
  await expect(dm.getByTestId("message-body")).toHaveAttribute(
    "data-message-bubble",
    "incoming",
  );
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
  await expect(
    dm
      .getByTestId("message-body")
      .filter({ hasText: "Absolutely — let's walk through it." }),
  ).toHaveAttribute("data-message-bubble", "outgoing");
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
  await expect(list).toHaveCSS("width", "220px");
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
  const outgoing = detail
    .getByTestId("message-row")
    .filter({ hasText: "A message from the split DM view" })
    .getByTestId("message-body");
  const incoming = detail
    .getByTestId("message-row")
    .filter({ hasText: "Got a few minutes" })
    .getByTestId("message-body");
  await expect(outgoing).toHaveAttribute("data-message-bubble", "outgoing");
  await expect(incoming).toHaveAttribute("data-message-bubble", "incoming");
  await expect(outgoing).toHaveCSS(
    "background-color",
    await themeColor(page, "--primary"),
  );
  await expect(outgoing).toHaveCSS(
    "color",
    await themeColor(page, "--primary-foreground"),
  );
  await outgoing.scrollIntoViewIfNeeded();
  const ownBounds = await outgoing.boundingBox();
  const rowBounds = await outgoing
    .locator("xpath=ancestor::article")
    .boundingBox();
  if (!ownBounds || !rowBounds) throw new Error("Message bounds unavailable");
  expect(
    Math.abs(ownBounds.x + ownBounds.width - (rowBounds.x + rowBounds.width)),
  ).toBeLessThan(12);
  await incoming.scrollIntoViewIfNeeded();
  const otherBounds = await incoming.boundingBox();
  if (!otherBounds) throw new Error("Incoming message bounds unavailable");
  expect(otherBounds.x - rowBounds.x).toBeLessThan(65);
  await outgoing.scrollIntoViewIfNeeded();
  expect(await detail.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(
    true,
  );
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
  await expect(list).toHaveCSS("width", "220px");
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
    .toBeLessThanOrEqual(960);
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
  await expect(page.getByTestId("pulse-tabs")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Agents", exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("app-top-chrome")
      .getByRole("button", { name: "App variations" }),
  ).toBeVisible();
  const list = page.getByTestId("pulse-combined-list");
  const detail = page.getByTestId("pulse-combined-detail");
  const all = list.getByRole("button", { name: "All messages", exact: true });
  const aggregate = page.getByTestId("pulse-all-messages-feed");
  await expect(list.getByRole("button").nth(0)).toHaveText("Search");
  await expect(list.getByRole("button").nth(1)).toHaveText("For you");
  await expect(list.getByRole("button").nth(2)).toHaveText("All messages");
  await list.getByRole("button", { name: "Search", exact: true }).click();
  await expect(
    page.getByRole("searchbox", { name: "Search loaded feed" }),
  ).toBeVisible();
  await expect(list).toBeVisible();
  await expect(all).not.toHaveAttribute("aria-current", "true");
  await list.getByRole("button", { name: "For you", exact: true }).click();
  await expect(page.getByTestId("pulse-briefing")).toBeVisible();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
  await all.click();
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
  await expect(list).toHaveCSS("width", "220px");
  await expect(page.getByTestId("pulse-main-container")).toHaveCSS(
    "max-width",
    "960px",
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
  await expect(page.getByTestId("pulse-tabs")).toHaveCount(0);
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

test("bubbles have equal corners, unwrapped media, and aligned reply indicators", async ({
  page,
}) => {
  await seed(page);
  await page.route("https://pulse-media.test/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="#dbeafe"/></svg>',
    }),
  );
  await page.getByRole("button", { name: "DMs", exact: true }).click();
  await page
    .getByTestId("pulse-dm-list")
    .locator('[data-channel-name="alice-tyler"]')
    .click();
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "alice-tyler",
    }),
  );
  await page.evaluate(
    async ({ alice }) => {
      const { pubkey: me } = await window.__TAURI_INTERNALS__.invoke<{
        pubkey: string;
      }>("get_identity");
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock bridge missing");
      const now = Math.floor(Date.now() / 1000);
      for (const [index, type] of ["image", "video"].entries()) {
        const url = `https://pulse-media.test/${type}.${type === "image" ? "png" : "mp4"}`;
        const root = emit({
          channelName: "alice-tyler",
          pubkey: me,
          content: `A ${type} caption${type === "video" ? "\n" : "\n\n"}![${type}](${url})`,
          createdAt: now + index * 2,
          extraTags: [
            [
              "imeta",
              `url ${url}`,
              `m ${type === "image" ? "image/png" : "video/mp4"}`,
              "dim 320x180",
            ],
          ],
        });
        emit({
          channelName: "alice-tyler",
          pubkey: alice,
          content: `Feedback on the ${type}`,
          parentEventId: root.id,
          createdAt: now + index * 2 + 1,
        });
      }
      const short = emit({
        channelName: "alice-tyler",
        pubkey: me,
        content: "Short text bubble",
        createdAt: now + 6,
      });
      emit({
        channelName: "alice-tyler",
        pubkey: alice,
        content: "Reply to short text",
        parentEventId: short.id,
        createdAt: now + 7,
      });
      emit({
        channelName: "alice-tyler",
        pubkey: alice,
        content: "![image](https://pulse-media.test/image.png)",
        createdAt: now + 8,
      });
      for (const [index, content] of [
        "First grouped bubble",
        "Second grouped bubble",
      ].entries()) {
        emit({
          channelName: "alice-tyler",
          pubkey: alice,
          content,
          createdAt: now + 9 + index,
        });
      }
      emit({
        channelName: "alice-tyler",
        pubkey: me,
        content: "A different sender",
        createdAt: now + 11,
      });
    },
    { alice: TEST_IDENTITIES.alice.pubkey },
  );
  const detail = page.getByTestId("pulse-dm-detail");
  for (const type of ["image", "video"]) {
    const row = detail
      .getByTestId("message-row")
      .filter({ hasText: `A ${type} caption` });
    const body = row.getByTestId("message-body");
    await body.scrollIntoViewIfNeeded();
    await expect(body).toHaveAttribute("data-message-bubble", "outgoing");
    await expect(body).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    await expect(body).toHaveCSS("padding", "0px");
    await expect(body.locator("[data-block-media]").first()).toBeVisible();
    const caption = body
      .locator("p, [data-bubble-caption]")
      .filter({ hasText: `A ${type} caption` });
    await expect(caption).toHaveCSS("border-radius", "20px");
    await expect(caption).toHaveCSS(
      "background-color",
      await themeColor(page, "--primary"),
    );
    const media = body
      .locator(
        type === "video"
          ? '[data-testid="video-player"]'
          : "[data-block-media] > button",
      )
      .first();
    const captionBounds = await caption.boundingBox();
    const mediaBounds = await media.boundingBox();
    const containerBounds = await body.boundingBox();
    if (!captionBounds || !mediaBounds || !containerBounds)
      throw new Error("Missing media bounds");
    expect(
      Math.abs(mediaBounds.y - captionBounds.y - captionBounds.height - 2),
    ).toBeLessThan(0.5);
    expect(
      Math.abs(
        mediaBounds.x +
          mediaBounds.width -
          containerBounds.x -
          containerBounds.width,
      ),
    ).toBeLessThan(1);
    const summary = row.getByTestId("message-thread-summary");
    await expect(summary).toBeVisible();
    const bodyBox = await body.boundingBox();
    const summaryBox = await summary.boundingBox();
    if (!bodyBox || !summaryBox) throw new Error("Missing message bounds");
    expect(Math.abs(bodyBox.x - summaryBox.x)).toBeLessThan(2);
  }
  const short = detail
    .getByTestId("message-row")
    .filter({ hasText: "Short text bubble" });
  const body = short.getByTestId("message-body");
  await body.scrollIntoViewIfNeeded();
  for (const corner of ["top-left", "top-right", "bottom-left", "bottom-right"])
    await expect(body).toHaveCSS(`border-${corner}-radius`, "20px");
  const bodyBox = await body.boundingBox();
  const summaryBox = await short
    .getByTestId("message-thread-summary")
    .boundingBox();
  if (!bodyBox || !summaryBox) throw new Error("Missing message bounds");
  expect(Math.abs(bodyBox.x - summaryBox.x)).toBeLessThan(2);
  const imageOnly = detail
    .locator('[data-message-bubble="incoming"]')
    .filter({ has: page.locator("[data-block-media]") });
  await imageOnly.scrollIntoViewIfNeeded();
  await expect(imageOnly).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  const firstGrouped = detail
    .getByTestId("message-body")
    .filter({ hasText: "First grouped bubble" });
  const secondGrouped = detail
    .getByTestId("message-body")
    .filter({ hasText: "Second grouped bubble" });
  await secondGrouped.scrollIntoViewIfNeeded();
  const firstBounds = await firstGrouped.boundingBox();
  const secondBounds = await secondGrouped.boundingBox();
  if (!firstBounds || !secondBounds)
    throw new Error("Missing grouped message bounds");
  expect(
    Math.abs(secondBounds.y - firstBounds.y - firstBounds.height - 2),
  ).toBeLessThan(0.5);
  const differentSender = detail
    .getByTestId("message-row")
    .filter({ hasText: "A different sender" });
  const groupStart = await differentSender.evaluate(
    (row) =>
      row.getBoundingClientRect().top +
      Number.parseFloat(getComputedStyle(row).paddingTop),
  );
  expect(
    Math.abs(groupStart - secondBounds.y - secondBounds.height - 24),
  ).toBeLessThan(0.5);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/13-bubble-media.png" });
  await short.getByTestId("message-thread-summary").click();
  await expect(detail.getByTestId("thread-view-mode-toggle")).toHaveCount(0);
  await expect(detail.getByTestId("chat-title")).toHaveCount(0);
  const back = page.getByRole("button", {
    name: "Back to conversation",
    exact: true,
  });
  await expect(back).toBeVisible();
  await expect(
    page.getByText("Reply to short text", { exact: true }),
  ).toBeVisible();
  await back.click();
  await expect(detail.getByTestId("chat-title")).toBeVisible();
});

test("live Pulse refresh and unread dots follow read state for channels and DMs", async ({
  page,
}) => {
  await seed(page);
  await page
    .getByRole("button", { name: "App variations", exact: true })
    .click();
  await page
    .getByRole("menuitemradio", { name: "Combined conversations", exact: true })
    .click();
  const list = page.getByTestId("pulse-combined-list");
  const detail = page.getByTestId("pulse-combined-detail");
  const all = list.getByRole("button", { name: "All messages", exact: true });
  for (const channelName of ["engineering", "alice-tyler"]) {
    const row = list.locator(`[data-channel-name="${channelName}"]`);
    await row.click();
    await expect(detail.getByTestId("message-input")).toBeVisible();
    await page.waitForFunction(
      (name) =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }),
      channelName,
    );
    await expect(row.getByTestId("pulse-unread-dot")).toHaveCount(0);
    await all.click();
    const text = `New unread message in ${channelName}`;
    await page.evaluate(
      ({ channelName, text, pubkey }) => {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: text,
          pubkey,
          createdAt: Math.floor(Date.now() / 1000) + 1,
        });
      },
      { channelName, text, pubkey: TEST_IDENTITIES.alice.pubkey },
    );
    await expect(row.getByTestId("pulse-unread-dot")).toBeVisible();
    await expect(row).toHaveAttribute("aria-description", "Unread messages");
    // This must update well before the old 30-second polling interval.
    await expect(
      page
        .getByTestId("pulse-all-messages-feed")
        .getByText(text, { exact: true }),
    ).toBeVisible({ timeout: 8_000 });
    await row.click();
    await expect(detail.getByText(text, { exact: true })).toBeVisible();
    await expect(row.getByTestId("pulse-unread-dot")).toHaveCount(0);
    await all.click();
    await expect(row.getByTestId("pulse-unread-dot")).toHaveCount(0);
    await page.evaluate(async (channelName) => {
      const me = await window.__TAURI_INTERNALS__.invoke<{ pubkey: string }>(
        "get_identity",
      );
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content: `My own activity in ${channelName}`,
        pubkey: me.pubkey,
        createdAt: Math.floor(Date.now() / 1000) + 2,
      });
    }, channelName);
    await expect(
      page
        .getByTestId("pulse-all-messages-feed")
        .getByText(`My own activity in ${channelName}`, { exact: true }),
    ).toBeVisible({ timeout: 8_000 });
    await expect(row.getByTestId("pulse-unread-dot")).toHaveCount(0);
  }
  await page.reload();
  await expect(list).toBeVisible();
  for (const name of ["engineering", "alice-tyler"]) {
    await expect(
      list
        .locator(`[data-channel-name="${name}"]`)
        .getByTestId("pulse-unread-dot"),
    ).toHaveCount(0);
  }
  await page
    .getByRole("button", { name: "Refresh messages", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Refresh messages", exact: true }),
  ).toBeEnabled();
  // The same read markers must drive the original, separate sidebars too.
  await page.evaluate((pubkey) => {
    for (const channelName of ["engineering", "alice-tyler"]) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        pubkey,
        content: `Unread in separate ${channelName}`,
        createdAt: Math.floor(Date.now() / 1000) + 3,
      });
    }
  }, TEST_IDENTITIES.alice.pubkey);
  await page
    .getByRole("button", { name: "App variations", exact: true })
    .click();
  await page
    .getByRole("menuitemradio", { name: "Separate feeds", exact: true })
    .click();
  await page.getByRole("button", { name: "Channels", exact: true }).click();
  const channelRow = page
    .getByTestId("pulse-channels-list")
    .locator('[data-channel-name="engineering"]');
  await expect(channelRow.getByTestId("pulse-unread-dot")).toBeVisible();
  await channelRow.click();
  await expect(channelRow.getByTestId("pulse-unread-dot")).toHaveCount(0);
  await page.getByRole("button", { name: "DMs", exact: true }).click();
  const dmRow = page
    .getByTestId("pulse-dm-list")
    .locator('[data-channel-name="alice-tyler"]');
  await dmRow.click();
  await expect(dmRow.getByTestId("pulse-unread-dot")).toHaveCount(0);
});

test("side panels expand Pulse and terminal docks beside the conversation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.addInitScript(() => {
    (window as Window & { isTauri: boolean }).isTauri = true;
  });
  await seed(page);
  // The browser bridge has no PTY backend. Stub only terminal IPC while the
  // actual shell, context selection, substrate, and panel controls run normally.
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async <T>(
      command: string,
      args?: Record<string, unknown>,
    ): Promise<T> => {
      if (command === "terminal_attach") {
        const request = args?.request as {
          channelId: string;
          columns: number;
          rows: number;
        };
        document.documentElement.dataset.terminalChannelId = request.channelId;
        return {
          sessionId: "pulse-test-terminal",
          subscriptionId: "pulse-test-subscription",
          viewport: {
            columns: request.columns,
            screenLines: request.rows,
            generation: 0,
          },
        } as T;
      }
      if (command === "terminal_resize")
        return {
          columns: args?.columns,
          screenLines: args?.rows,
          generation: 1,
        } as T;
      if (command.startsWith("terminal_")) return undefined as T;
      return invoke<T>(command, args);
    };
  });
  await page.getByRole("button", { name: "Channels", exact: true }).click();
  await page
    .getByTestId("pulse-channels-list")
    .getByRole("button", { name: "engineering", exact: true })
    .click();
  const container = page.getByTestId("pulse-main-container");
  const detail = page.getByTestId("pulse-channel-detail");
  await expect(container).toHaveCSS("max-width", "960px");
  await detail
    .getByRole("button", { name: "Open Buzz Term", exact: true })
    .click();
  const terminal = page.getByRole("region", { name: "Buzz Term" });
  await expect(terminal).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute(
    "data-terminal-channel-id",
    (await detail.getAttribute("data-channel-id")) ?? "",
  );
  await expect(container).toHaveCSS("max-width", "none");
  await expect
    .poll(async () => (await boundsOf(container)).width)
    .toBeGreaterThan(1400);
  const conversationBounds = await boundsOf(detail);
  const terminalBounds = await boundsOf(terminal);
  expect(terminalBounds.x).toBeGreaterThanOrEqual(
    conversationBounds.x + conversationBounds.width - 1,
  );
  expect(terminalBounds.height).toBeGreaterThan(700);
  const expandedBounds = await boundsOf(container);
  expect(expandedBounds.x).toBe(8);
  expect(expandedBounds.width).toBe(1584);
  const chatHeader = detail.getByTestId("chat-header");
  const terminalHeader = terminal.getByTestId("terminal-header");
  expect((await boundsOf(terminalHeader)).height).toBe(
    (await boundsOf(chatHeader)).height,
  );
  const headerButtons = detail.locator(
    "[data-conversation-header-actions] button",
  );
  for (const button of await headerButtons.all()) {
    await expect(button).toHaveCSS("border-width", "0px");
  }
  await expect(detail.locator("[data-conversation-header-actions]")).toHaveCSS(
    "gap",
    "0px",
  );
  await expect(detail.locator("[data-channel-header-controls]")).toHaveCSS(
    "gap",
    "0px",
  );
  await expect(
    terminal.getByRole("separator", { name: "Resize Buzz Term" }),
  ).toHaveCount(0);
  await terminal.getByRole("button", { name: "Maximize Buzz Term" }).click();
  await expect(page.getByTestId("pulse-scroll-area")).toBeHidden();
  await terminal.getByRole("button", { name: "Restore Buzz Term" }).click();
  await expect(detail).toBeVisible();
  await terminal.getByRole("button", { name: "Hide Buzz Term" }).click();
  await expect(container).toHaveCSS("max-width", "960px");
  await detail.getByTestId("channel-management-trigger").click();
  const settings = page.getByTestId("channel-management-sheet");
  await expect(settings).toBeVisible();
  await expect(container).toHaveCSS("max-width", "none");
  await expect(
    page.getByTestId("channel-management-auxiliary-pane"),
  ).toBeVisible();
  await waitForAnimations(page);
  await container.screenshot({
    path: "test-results/pulse-prototype/15-expanded-settings.png",
  });
  await settings.getByRole("button", { name: /close/i }).first().click();
  await expect(container).toHaveCSS("max-width", "960px");
  await detail
    .getByRole("button", { name: "Open Buzz Term", exact: true })
    .click();
  await expect(terminal).toBeVisible();
  await waitForAnimations(page);
  await container.screenshot({
    path: "test-results/pulse-prototype/16-terminal-side-panel.png",
  });
  await page.keyboard.press("Control+j");
  await expect(container).toHaveCSS("max-width", "960px");
});

test("bubbles follow the theme and anchor reactions and hover controls without row fill", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("buzz-theme", "github-light");
    localStorage.setItem("buzz-accent-color", "#a855f7");
    localStorage.setItem("buzz-glass-background", "true");
  });
  await seed(page);
  await page.getByRole("button", { name: "DMs", exact: true }).click();
  await page
    .getByTestId("pulse-dm-list")
    .locator('[data-channel-name="alice-tyler"]')
    .click();
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "alice-tyler",
    }),
  );
  await page.evaluate(
    ({ alice }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock emitter missing");
      const message = emit({
        channelName: "alice-tyler",
        content: "Yes",
        pubkey: alice,
      });
      emit({
        channelName: "alice-tyler",
        content: "👍",
        kind: 7,
        extraTags: [["e", message.id]],
      });
      emit({ channelName: "alice-tyler", content: "Theme-colored message" });
    },
    { alice: TEST_IDENTITIES.alice.pubkey },
  );
  const detail = page.getByTestId("pulse-dm-detail");
  const own = detail
    .getByTestId("message-row")
    .filter({ hasText: "Theme-colored message" });
  await expect(own.getByTestId("message-body")).toHaveCSS(
    "background-color",
    await themeColor(page, "--primary"),
  );
  expect(await themeColor(page, "--primary")).toBe("rgb(168, 85, 247)");
  await expect(page.getByTestId("unified-pulse")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(
    page.getByTestId("unified-pulse").locator("xpath=..").locator("xpath=.."),
  ).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  const row = detail
    .getByTestId("message-row")
    .filter({ has: page.getByText("Yes", { exact: true }) });
  const reaction = row.getByLabel("Toggle 👍 reaction");
  await expect(reaction).toBeVisible();
  await row.scrollIntoViewIfNeeded();
  const body = await boundsOf(row.getByTestId("message-body"));
  const badge = await boundsOf(row.getByTestId("bubble-reactions-anchor"));
  expect(
    Math.abs(badge.x + badge.width - body.x - body.width - 8),
  ).toBeLessThan(1);
  expect(Math.abs(badge.y + badge.height / 2 - body.y)).toBeLessThan(1);
  await row.hover();
  await expect(row).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  const controls = row.getByTestId("bubble-actions-anchor");
  await expect(controls.getByLabel("Open reactions")).toBeVisible();
  const bounds = await boundsOf(controls);
  expect(bounds.x).toBeGreaterThanOrEqual((await boundsOf(row)).x);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(badge.y);
  await controls.getByLabel("Open reactions").click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await waitForAnimations(page);
  await detail.screenshot({
    path: "test-results/pulse-prototype/17-theme-reaction-corner.png",
  });
});
