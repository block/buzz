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
  ).toHaveAttribute("aria-current", "true");
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
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  await expect(briefing).toHaveCount(0);
});

test("mixed feed, search, and DM reply destination", async ({ page }) => {
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
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  const list = page.getByTestId("pulse-combined-list");
  await expect(list).toHaveCSS("width", "220px");
  const detail = page.getByTestId("pulse-combined-detail");
  const alice = list.locator('[data-channel-name="alice-tyler"]');
  const bob = list.locator('[data-channel-name="bob-tyler"]');
  await expect(alice).toHaveCount(1);
  await expect(bob).toHaveCount(1);
  await expect(alice).toContainText("Alice");
  await expect(
    alice.getByTestId("pulse-conversation-presence"),
  ).toHaveAttribute("aria-label", "Online");
  await expect(bob.getByTestId("pulse-conversation-presence")).toHaveAttribute(
    "aria-label",
    "Away",
  );
  await expect(list).not.toContainText("Got a few minutes");
  await alice.click();
  await expect(alice).toHaveAttribute("aria-current", "true");
  await expect(detail).toContainText("Got a few minutes");
  await expect(detail.getByTestId("chat-title")).toContainText(/alice/i);
  await expect(detail.getByTestId("pulse-message-view")).toHaveCSS(
    "padding-bottom",
    "0px",
  );
  const composer = detail.getByTestId("message-composer");
  await expect(composer).toHaveCSS("border-width", "0px");
  await expect(composer).not.toHaveCSS("box-shadow", "none");
  await expect
    .poll(async () => {
      const box = await boundsOf(composer);
      const pane = await boundsOf(detail);
      return [
        Math.round(box.x - pane.x),
        Math.round(pane.x + pane.width - box.x - box.width),
        Math.round(pane.y + pane.height - box.y - box.height),
      ];
    })
    .toEqual([8, 8, 8]);
  await expect
    .poll(() =>
      detail.getByTestId("message-timeline").evaluate((el) => {
        const overlay = el
          .closest('[data-testid="pulse-message-view"]')
          ?.querySelector('[data-testid="channel-composer-overlay"]');
        const reserved = parseFloat(
          getComputedStyle(el).getPropertyValue("--composer-overlay-height"),
        );
        return Math.round(
          reserved - (overlay?.getBoundingClientRect().height ?? 0),
        );
      }),
    )
    .toBe(16);
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
  await expect(page.getByTestId("pulse-combined-detail")).toHaveCount(0);
  await expect(page.getByTestId("pulse-briefing")).toBeVisible();
  await expect(page.getByTestId("pulse-conversation")).toHaveCount(0);
});

test("channel rail opens conversations and preserves drafts", async ({
  page,
}) => {
  await seed(page);
  await page
    .getByTestId("unified-pulse")
    .getByRole("button", { name: "All messages", exact: true })
    .click();
  const list = page.getByTestId("pulse-combined-list");
  await expect(list).toHaveCSS("width", "220px");
  const all = list.getByRole("button", { name: "All messages", exact: true });
  const aggregate = page.getByTestId("pulse-all-messages-feed");
  await expect(list.getByRole("button").nth(2)).toHaveText("All messages");
  await expect(all).toHaveAttribute("aria-current", "true");
  await expect(
    aggregate
      .getByTestId("pulse-conversation")
      .filter({ hasText: "What if catching up" }),
  ).toBeVisible();
  await expect(aggregate).toContainText("Got a few minutes");
  await expect(list.locator('[data-channel-name="alice-tyler"]')).toHaveCount(
    1,
  );
  await expect
    .poll(
      async () =>
        (await page.getByTestId("pulse-combined-view").boundingBox())?.width,
    )
    .toBeLessThanOrEqual(960);
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/05-all-channels.png" });
  await list
    .getByRole("button", { name: "Open channel engineering", exact: true })
    .click();
  const detail = page.getByTestId("pulse-combined-detail");
  await expect(detail.getByTestId("chat-title")).toHaveText("engineering");
  await expect(detail).toContainText("Finished reviewing the relay changes");
  await detail.getByTestId("message-input").fill("Draft for engineering");
  await list
    .getByRole("button", { name: "Open channel random", exact: true })
    .click();
  await expect(detail.getByTestId("chat-title")).toHaveText("random");
  await expect(detail.getByTestId("message-input")).toHaveText("");
  await list
    .getByRole("button", { name: "Open channel engineering", exact: true })
    .click();
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
    .getByRole("button", { name: "Open channel announcements", exact: true })
    .click();
  await expect(detail.getByTestId("chat-title")).toHaveText("announcements");
  await expect(page).toHaveURL(/#\/pulse/);
});

test("conversation rail stays visible while the feed scrolls and after reload", async ({
  page,
}) => {
  await seed(page);
  const segments = page.getByTestId("pulse-combined-list");
  await expect(segments.getByRole("button").first()).toHaveText("Search");
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/09-summary-only.png" });
  const search = segments.getByRole("button", { name: "Search", exact: true });
  await search.focus();
  await page.keyboard.press("Enter");
  await expect(search).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("pulse-conversation").first()).toBeVisible();
  const before = await segments.boundingBox();
  const scroll = page.getByTestId("pulse-search-feed");
  await scroll.evaluate((el) => el.scrollTo({ top: el.scrollHeight }));
  await expect
    .poll(() => scroll.evaluate((el) => el.scrollTop))
    .toBeGreaterThan(0);
  await expect
    .poll(async () => (await segments.boundingBox())?.y)
    .toBe(before?.y);
  await expect(search).toBeInViewport();
  await page.reload();
  await expect(page.getByTestId("pulse-combined-list")).toBeVisible();
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
  await expect(page.getByTestId("pulse-combined-list")).toBeInViewport();
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
    page.getByRole("menuitemradio", {
      name: "Combined conversations",
      exact: true,
    }),
  ).toHaveAttribute("aria-checked", "true");
  await page
    .getByRole("menuitemradio", { name: "Combined conversations", exact: true })
    .click();
  await expect(page.getByTestId("pulse-tabs")).toHaveCount(0);
  await expect(
    page
      .getByTestId("pulse-app-navigation")
      .getByRole("button", { name: "Agents", exact: true }),
  ).toBeVisible();
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
  await expect(page.getByTestId("pulse-tabs")).toHaveCount(0);
  await expect(
    page.getByTestId("pulse-conversation-group-divider"),
  ).toHaveCount(1);
  await expect(detail).toHaveAttribute(
    "data-channel-id",
    engineeringId ?? "missing",
  );
  const divider = page.getByTestId("pulse-conversation-group-divider");
  expect((await boundsOf(alice)).y).toBeLessThan((await boundsOf(divider)).y);
  expect((await boundsOf(engineering)).y).toBeGreaterThan(
    (await boundsOf(divider)).y,
  );
  await page.reload();
  await expect(divider).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/18-separate-rail.png" });
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
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  await page
    .getByTestId("pulse-combined-list")
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
  const detail = page.getByTestId("pulse-combined-detail");
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
  const secondGrouped = detail
    .getByTestId("message-body")
    .filter({ hasText: "Second grouped bubble" });
  await secondGrouped.scrollIntoViewIfNeeded();
  await expect(
    detail
      .getByTestId("message-row")
      .filter({ hasText: "First grouped bubble" })
      .getByTestId("message-avatar"),
  ).toHaveCount(0);
  await expect(
    detail
      .getByTestId("message-row")
      .filter({ hasText: "Second grouped bubble" })
      .getByTestId("message-avatar"),
  ).toBeVisible();
  // Read related bounds in one frame: the virtualizer may still be settling
  // scrollTop after scrollIntoView, which invalidates cross-frame comparisons.
  const geometry = await detail
    .getByTestId("message-row")
    .evaluateAll((rows) => {
      const rowFor = (text: string) => {
        const row = rows.find((row) => row.textContent?.includes(text));
        if (!row) throw new Error(`Missing row: ${text}`);
        return row;
      };
      const bodyFor = (text: string) => {
        const body = rowFor(text).querySelector('[data-testid="message-body"]');
        if (!body) throw new Error(`Missing bubble: ${text}`);
        return body.getBoundingClientRect();
      };
      const first = bodyFor("First grouped bubble");
      const last = bodyFor("Second grouped bubble");
      const avatar = rowFor("Second grouped bubble").querySelector(
        '[data-testid="message-avatar"]',
      );
      if (!avatar) throw new Error("Missing final avatar");
      const next = rowFor("A different sender");
      return {
        sameAuthorGap: last.top - first.bottom,
        avatarOffset: avatar.getBoundingClientRect().bottom - last.bottom,
        differentAuthorGap:
          next.getBoundingClientRect().top +
          Number.parseFloat(getComputedStyle(next).paddingTop) -
          last.bottom,
      };
    });
  expect(geometry.sameAuthorGap).toBeCloseTo(2, 1);
  expect(geometry.avatarOffset).toBeCloseTo(0, 1);
  expect(geometry.differentAuthorGap).toBeCloseTo(24, 1);
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
  const threadComposer = detail.getByTestId("message-composer");
  await expect(threadComposer).toHaveCSS("border-width", "0px");
  await expect
    .poll(async () => {
      const box = await boundsOf(threadComposer);
      const pane = await boundsOf(detail);
      return [
        Math.round(box.x - pane.x),
        Math.round(pane.x + pane.width - box.x - box.width),
        Math.round(pane.y + pane.height - box.y - box.height),
      ];
    })
    .toEqual([8, 8, 8]);
  await expect
    .poll(() =>
      detail.getByTestId("message-thread-body").evaluate((el) => {
        const overlay = el
          .closest('[data-testid="pulse-message-view"]')
          ?.querySelector('[data-testid="thread-composer-overlay"]');
        return Math.round(
          parseFloat(getComputedStyle(el).paddingBottom) -
            (overlay?.getBoundingClientRect().height ?? 0),
        );
      }),
    )
    .toBe(16);
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
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  const channelRow = page
    .getByTestId("pulse-combined-list")
    .locator('[data-channel-name="engineering"]');
  await expect(channelRow.getByTestId("pulse-unread-dot")).toBeVisible();
  await channelRow.click();
  await expect(channelRow.getByTestId("pulse-unread-dot")).toHaveCount(0);
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  const dmRow = page
    .getByTestId("pulse-combined-list")
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
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  await page
    .getByTestId("pulse-combined-list")
    .getByRole("button", { name: "Open channel engineering", exact: true })
    .click();
  const container = page.getByTestId("pulse-main-container");
  const detail = page.getByTestId("pulse-combined-detail");
  await expect(container).toHaveCSS("max-width", "960px");
  await expect(container).toHaveCSS("border-width", "0px");
  await detail
    .getByRole("button", { name: "Open Buzz Term", exact: true })
    .click();
  const terminal = page.getByRole("region", { name: "Buzz Term" });
  await expect(terminal).toBeVisible();
  await expect(terminal).toHaveCSS("transition-property", "transform, opacity");
  await expect(terminal).toHaveCSS("transition-duration", "0.18s, 0.18s");
  await waitForAnimations(page);
  await expect(page.locator("html")).toHaveAttribute(
    "data-terminal-channel-id",
    (await detail.getAttribute("data-channel-id")) ?? "",
  );
  await expect(container).toHaveCSS("max-width", "none");
  await expect
    .poll(async () => (await boundsOf(container)).width)
    .toBeGreaterThan(1300);
  const conversationBounds = await boundsOf(detail);
  const terminalBounds = await boundsOf(terminal);
  expect(terminalBounds.x).toBeGreaterThanOrEqual(
    conversationBounds.x + conversationBounds.width - 1,
  );
  expect(terminalBounds.height).toBeGreaterThan(700);
  const expandedBounds = await boundsOf(container);
  const appsBounds = await boundsOf(page.getByTestId("pulse-app-navigation"));
  expect(expandedBounds.x).toBe(appsBounds.x + appsBounds.width + 8);
  expect(expandedBounds.x + expandedBounds.width).toBe(1592);
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
    await expect(button).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
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
  // Record a frame during exit: the closing panel retains its space, becomes
  // non-interactive, and fades before it is removed and the card contracts.
  await page.evaluate(() => {
    const host = document.querySelector('[data-testid="pulse-main-container"]');
    const panel = host?.querySelector('[data-terminal-side-panel="true"]');
    if (!host || !panel) throw new Error("Missing terminal panel");
    (window as unknown as { terminalExit: Promise<unknown> }).terminalExit =
      new Promise((resolve) => {
        panel.addEventListener(
          "transitionrun",
          () => {
            resolve({
              inert: panel.hasAttribute("inert"),
              width: getComputedStyle(host).maxWidth,
              opacity: parseFloat(getComputedStyle(panel).opacity),
            });
          },
          { once: true },
        );
      });
  });
  await terminal.getByRole("button", { name: "Hide Buzz Term" }).click();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { terminalExit: Promise<unknown> }).terminalExit,
    ),
  ).toMatchObject({ inert: true, width: "none", opacity: expect.any(Number) });
  await expect(terminal).toHaveCount(0);
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
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(terminal).toHaveCSS("transform", "none");
  await expect(terminal).toHaveCSS("transition-property", "opacity");
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
  await page.getByRole("button", { name: "All messages", exact: true }).click();
  await page
    .getByTestId("pulse-combined-list")
    .locator('[data-channel-name="alice-tyler"]')
    .click();
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "alice-tyler",
    }),
  );
  const linkIds = await page.evaluate(
    ({ alice }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock emitter missing");
      return [alice, undefined].map(
        (pubkey) =>
          emit({
            channelName: "alice-tyler",
            content: "https://example.com/accessible-link",
            pubkey,
          }).id,
      );
    },
    { alice: TEST_IDENTITIES.alice.pubkey },
  );
  for (const id of linkIds) {
    const message = page
      .getByTestId("pulse-combined-detail")
      .getByTestId("message-row")
      .filter({ has: page.getByTestId(`message-action-bar-${id}`) });
    await message.scrollIntoViewIfNeeded();
    await message.hover();
    const bar = message
      .getByTestId(`message-action-bar-${id}`)
      .locator(":scope > div");
    await expect(bar).toBeVisible();
    const bubbleBox = await boundsOf(message.getByTestId("message-body"));
    const barBox = await boundsOf(bar);
    expect(barBox.y + barBox.height - bubbleBox.y).toBeCloseTo(4, 0);
    await message
      .getByRole("link", {
        name: "https://example.com/accessible-link",
        exact: true,
      })
      .click({ trial: true });
  }
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
      const own = emit({
        channelName: "alice-tyler",
        content: "Theme-colored message",
      });
      emit({
        channelName: "alice-tyler",
        content: "👍",
        kind: 7,
        pubkey: alice,
        extraTags: [["e", own.id]],
      });
      for (const target of [message, own]) {
        emit({
          channelName: "alice-tyler",
          content: "❤️",
          kind: 7,
          pubkey: alice,
          extraTags: [["e", target.id]],
        });
      }
    },
    { alice: TEST_IDENTITIES.alice.pubkey },
  );
  const detail = page.getByTestId("pulse-combined-detail");
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
  const ownReaction = own.getByLabel("Toggle 👍 reaction");
  await expect(ownReaction).toBeVisible();
  const ownBody = await boundsOf(own.getByTestId("message-body"));
  const ownBadge = await boundsOf(own.getByTestId("bubble-reactions-anchor"));
  expect(Math.abs(ownBadge.x - ownBody.x + 8)).toBeLessThan(1);
  expect(Math.abs(ownBadge.y + ownBadge.height / 2 - ownBody.y)).toBeLessThan(
    1,
  );
  for (const message of [row, own]) {
    const pill = message.getByLabel("Toggle 👍 reaction");
    const group = message.getByRole("group", { name: "Message reactions" });
    await expect(group.getByRole("button")).toHaveCount(2);
    await expect(group).toHaveCSS("gap", "0px");
    await expect(group).toHaveCSS("border-radius", "9999px");
    await expect(pill).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    await expect(pill).toHaveCSS("border-width", "0px");
    expect(
      await group
        .locator(".reaction-segment")
        .nth(1)
        .evaluate((el) => {
          const divider = getComputedStyle(el, "::before");
          return {
            width: divider.width,
            height: divider.height,
            background: divider.backgroundColor,
          };
        }),
    ).toMatchObject({
      width: "1px",
      background: expect.stringMatching(/0\.15\)/),
    });
    await expect(group).toHaveCSS(
      "background-color",
      await message
        .getByTestId("message-body")
        .evaluate((el) => getComputedStyle(el).backgroundColor),
    );
    await expect(group).toHaveCSS(
      "border-color",
      await themeColor(page, "--background"),
    );
    // Browsers quantize border widths to physical pixels (1px at DPR 1).
    expect(
      await group.evaluate((el) =>
        parseFloat(getComputedStyle(el).borderTopWidth),
      ),
    ).toBe(
      await page.evaluate(
        () => Math.floor(1.5 * devicePixelRatio) / devicePixelRatio,
      ),
    );
    await expect(pill).toHaveCSS("font-size", "10px");
    await expect(pill.locator("[data-reaction-glyph]")).toHaveCSS(
      "font-size",
      "10px",
    );
    await expect(pill.locator(".buzz-animated-count")).toHaveCSS(
      "color",
      await message
        .getByTestId("message-body")
        .evaluate((el) => getComputedStyle(el).color),
    );
  }
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
  await ownReaction.click();
  await expect(ownReaction).toHaveAttribute("aria-pressed", "true");
  await expect(own.getByLabel("Toggle ❤️ reaction")).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await ownReaction.press("Enter");
  await expect(ownReaction).toHaveAttribute("aria-pressed", "false");
  await expect(
    own.getByRole("group", { name: "Message reactions" }).getByRole("button"),
  ).toHaveCount(2);
});

test("typing fades inside conversation and thread scroll areas without moving the composer", async ({
  page,
}) => {
  await seed(page);
  await page.clock.install();
  const list = page.getByTestId("pulse-combined-list");
  const detail = page.getByTestId("pulse-combined-detail");
  for (const channelName of ["bob-tyler", "random"]) {
    await list.locator(`[data-channel-name="${channelName}"]`).click();
    await expect(detail.getByTestId("message-input")).toBeVisible();
    await page.waitForFunction(
      (channelName) =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName,
          kind: 20002,
        }),
      channelName,
    );
    await page.evaluate(
      ({ channelName, pubkey }) => {
        const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        for (const [index, content] of [
          "Bob's first bubble",
          "Bob's final bubble",
        ].entries()) {
          emit?.({
            channelName,
            pubkey,
            content,
            createdAt: Math.floor(Date.now() / 1000) - 10 + index,
          });
        }
      },
      { channelName, pubkey: TEST_IDENTITIES.bob.pubkey },
    );
    const scroll = detail.getByTestId("message-timeline");
    const finalRow = scroll
      .getByTestId("message-row")
      .filter({ hasText: "Bob's final bubble" });
    const lastAvatar = finalRow.getByTestId("message-avatar");
    await expect(lastAvatar).toBeVisible();
    await lastAvatar.scrollIntoViewIfNeeded();
    await page.clock.fastForward(3_000);
    await waitForAnimations(page);
    const avatarBefore = await boundsOf(lastAvatar);
    const indicator = scroll.getByTestId("timeline-typing-transition");
    const composer = detail.getByTestId("message-composer");
    await expect(indicator).toHaveAttribute("data-active", "false");
    const before = await boundsOf(composer);
    const scrollBefore = await scroll.evaluate((el) => ({
      height: el.scrollHeight,
      top: el.scrollTop,
    }));
    const avatarFrames = await page.evaluate(
      async ({ channelName, pubkey }) => {
        window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({ channelName, pubkey });
        const positions: number[] = [];
        for (let frame = 0; frame < 18; frame++) {
          await new Promise(requestAnimationFrame);
          const avatar = document.querySelector(
            '[data-testid="message-typing-avatar"] [data-testid="bubble-avatar"]',
          );
          if (avatar) positions.push(avatar.getBoundingClientRect().y);
        }
        return positions;
      },
      { channelName, pubkey: TEST_IDENTITIES.bob.pubkey },
    );
    expect(
      Math.max(...avatarFrames) - Math.min(...avatarFrames),
    ).toBeGreaterThan(3);
    await expect(indicator).toHaveAttribute("data-active", "true");
    await expect(scroll.getByRole("status")).toContainText("Bob is typing");
    await expect(indicator.locator(".typing-dot")).toHaveCount(3);
    await expect(
      indicator.getByTestId("message-typing-indicator-label"),
    ).toHaveCount(0);
    await expect(indicator).toHaveCSS("opacity", "1");
    await expect(lastAvatar).toHaveCount(0);
    const typingAvatar = scroll.getByTestId("message-typing-avatar");
    await expect(typingAvatar).toHaveCount(1);
    await waitForAnimations(page);
    const movedAvatar = await boundsOf(typingAvatar);
    const typingBubble = await boundsOf(indicator);
    expect(movedAvatar.y).toBeGreaterThan(avatarBefore.y);
    expect(
      Math.abs(
        movedAvatar.y +
          movedAvatar.height -
          typingBubble.y -
          typingBubble.height,
      ),
    ).toBeLessThan(1);
    await expect(indicator).toHaveCSS("transition-duration", "0.18s, 0.18s");
    await expect(
      detail
        .getByTestId("channel-composer-overlay")
        .getByTestId("message-typing-indicator"),
    ).toHaveCount(0);
    expect(await boundsOf(composer)).toEqual(before);
    expect(
      await scroll.evaluate((el) => ({
        height: el.scrollHeight,
        top: el.scrollTop,
      })),
    ).toEqual(scrollBefore);
    const typingBox = await boundsOf(indicator);
    expect(typingBox.y + typingBox.height).toBeLessThan(before.y);
    await waitForAnimations(page);
    await detail.screenshot({
      path: `test-results/pulse-prototype/typing-${channelName}.png`,
    });
    if (channelName === "random") {
      await page.evaluate(
        ({ pubkey }) =>
          window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
            channelName: "random",
            pubkey,
          }),
        { pubkey: TEST_IDENTITIES.outsider.pubkey },
      );
      await expect(typingAvatar).toHaveCount(2);
      await expect(scroll.locator(".typing-dots-bubble")).toHaveCount(1);
      await waitForAnimations(page);
      const first = await boundsOf(typingAvatar.first());
      const second = await boundsOf(typingAvatar.last());
      expect(first.width).toBe(20);
      expect(second.x).toBeLessThan(first.x + first.width);
      await expect(
        typingAvatar.first().locator(".typing-avatar-stroke"),
      ).toHaveCSS("box-shadow", /1.5px/);
      await detail.screenshot({
        path: "test-results/pulse-prototype/typing-grouped.png",
      });
    }
    await page.clock.fastForward(9_000);
    await expect(indicator).toHaveAttribute("data-active", "false");
    await expect(indicator).toHaveCSS("opacity", "0");
    await expect(indicator).toHaveAttribute("aria-hidden", "true");
    await expect(lastAvatar).toBeVisible();
    await waitForAnimations(page);
    expect(await boundsOf(lastAvatar)).toEqual(avatarBefore);
    expect(await boundsOf(composer)).toEqual(before);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(indicator).toHaveCSS("transform", "none");
    await expect(indicator).toHaveCSS("transition-property", "opacity");
    await expect(indicator.locator(".typing-dot").first()).toHaveCSS(
      "animation-name",
      "none",
    );
    await page.emulateMedia({ reducedMotion: "no-preference" });
  }
  const root = detail
    .getByTestId("message-row")
    .filter({ hasText: "What if catching up" });
  await root.getByTestId("message-thread-summary").click();
  const body = detail.getByTestId("message-thread-body");
  await expect(body).toBeVisible();
  await waitForAnimations(page);
  const headId = new URLSearchParams((await page.url()).split("?")[1]).get(
    "thread",
  );
  if (!headId) throw new Error("Missing selected thread");
  const indicator = body.getByTestId("timeline-typing-transition");
  const composer = detail.getByTestId("message-composer");
  const before = await boundsOf(composer);
  await page.evaluate(
    ({ pubkey, threadHeadId }) =>
      window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
        channelName: "random",
        pubkey,
        threadHeadId,
      }),
    { pubkey: TEST_IDENTITIES.bob.pubkey, threadHeadId: headId },
  );
  await expect(indicator).toHaveAttribute("data-active", "true");
  await expect(indicator).toHaveCSS("opacity", "1");
  await expect(
    detail
      .getByTestId("thread-composer-overlay")
      .getByTestId("message-typing-indicator"),
  ).toHaveCount(0);
  expect(await boundsOf(composer)).toEqual(before);
  await page
    .getByRole("button", { name: "Back to conversation", exact: true })
    .click();
  await expect(
    detail
      .getByTestId("message-timeline")
      .getByTestId("timeline-typing-transition"),
  ).toHaveAttribute("data-active", "false");
});

test("workspace entrypoints keep projects, agents, and workflows in the main panel", async ({
  page,
}) => {
  await seed(page);
  const rail = page.getByTestId("pulse-combined-list");
  const main = page.getByTestId("pulse-main-container");
  const apps = page.getByTestId("pulse-app-navigation");
  await expect(main.getByTestId("pulse-app-heading")).toHaveCount(0);
  await expect(
    apps.getByRole("button", { name: "Messages", exact: true }),
  ).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(apps.locator("button > svg")).toHaveCount(4);
  for (const width of [1600, 1280, 900]) {
    await page.setViewportSize({ width, height: 960 });
    const box = await boundsOf(main);
    expect(box.x + box.width / 2).toBeCloseTo(width / 2, 0);
    const navBox = await boundsOf(apps);
    expect(box.x).toBeGreaterThanOrEqual(navBox.x + navBox.width);
  }
  await page.setViewportSize({ width: 1440, height: 960 });
  await expect(
    rail
      .getByRole("button")
      .filter({ has: page.locator("svg") })
      .first(),
  ).toHaveText("Search");
  await expect(rail.getByRole("button").nth(1)).toHaveText("For you");
  await expect(apps.getByRole("button")).toHaveText([
    "Messages",
    "Projects",
    "Agents",
    "Workflows",
  ]);
  for (const name of ["Projects", "Agents", "Workflows"]) {
    await expect(rail.getByRole("button", { name, exact: true })).toHaveCount(
      0,
    );
  }
  for (const variation of ["Combined conversations", "Separate feeds"]) {
    await page
      .getByRole("button", { name: "App variations", exact: true })
      .click();
    await page
      .getByRole("menuitemradio", { name: variation, exact: true })
      .click();
    for (const [name, contentTestId] of [
      ["Projects", "projects-overview-layout"],
      ["Agents", "agents-page-content"],
      ["Workflows", "workflows-view"],
    ]) {
      const entry = apps.getByRole("button", { name, exact: true });
      await entry.focus();
      await page.keyboard.press("Enter");
      const panel = main.getByTestId(`pulse-workspace-${name.toLowerCase()}`);
      await expect(panel.getByTestId(contentTestId)).toBeVisible();
      await expect(entry).toHaveAttribute("aria-current", "page");
      await expect(apps).toBeVisible();
      await expect(main).toHaveCSS("max-width", "960px");
      await expect(page).toHaveURL(
        new RegExp(`#/pulse\\?.*feed=${name.toLowerCase()}`),
      );
      await expect(page.getByTestId("open-pulse-view")).toHaveCount(0);
      await expect(rail).toHaveCount(0);
      await expect(main.getByTestId("pulse-app-heading")).toHaveCount(0);
      const appBox = await boundsOf(apps);
      const mainBox = await boundsOf(main);
      const panelBox = await boundsOf(panel);
      expect(mainBox.x).toBeGreaterThanOrEqual(appBox.x + appBox.width + 8);
      expect(panelBox.width).toBeCloseTo(mainBox.width, 0);
      expect(mainBox.x + mainBox.width / 2).toBeCloseTo(720, 0);
      await page.reload();
      await expect(panel.getByTestId(contentTestId)).toBeVisible();
      await expect(entry).toHaveAttribute("aria-current", "page");
    }
  }
  await main
    .getByRole("button", { name: "Create Workflow", exact: true })
    .click();
  const editor = page.getByRole("dialog", { name: "Create workflow" });
  await expect(editor).toBeVisible();
  await expect(page).toHaveURL(/#\/pulse\?.*view=create/);
  await expect(apps).toBeVisible();
  await page.goBack();
  await expect(editor).toHaveCount(0);
  await expect(main.getByTestId("workflows-view")).toBeVisible();
  await apps.getByRole("button", { name: "Projects", exact: true }).click();
  const projectsRail = main.getByTestId("pulse-projects-list");
  await expect(projectsRail).toBeVisible();
  await expect(projectsRail).toHaveCSS("width", "220px");
  for (const section of [
    "repositories",
    "issues",
    "prs",
    "channels",
    "activity",
  ]) {
    const row = projectsRail.getByTestId(`projects-section-${section}`);
    await row.click();
    await expect(row).toHaveAttribute("aria-current", "page");
    await expect(main.getByTestId("projects-panel-toolbar")).toBeVisible();
  }
  await expect(main.getByTestId("projects-page-tabs")).toHaveCount(0);
  await expect(main.getByTestId("projects-overview-context-rail")).toHaveCount(
    0,
  );
  await projectsRail.getByTestId("projects-section-projects").click();
  await expect(
    projectsRail.getByTestId("projects-section-projects"),
  ).toHaveAttribute("aria-current", "page");
  const search = main.getByRole("searchbox", {
    name: "Search Projects",
    exact: true,
  });
  await search.fill("no-such-project");
  await expect(
    main.locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    ),
  ).toHaveCount(0);
  await search.clear();
  await main
    .getByRole("button", { name: "Create project", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const project = main.locator(
    '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
  );
  await expect(project).toBeVisible();
  await project.click();
  await expect(page).toHaveURL(/#\/pulse\?.*projectId=/);
  await expect(projectsRail).toBeVisible();
  await expect(
    projectsRail.getByRole("button", {
      name: "Open project buzz",
      exact: true,
    }),
  ).toHaveAttribute("aria-current", "page");
  await waitForAnimations(page);
  await expect(
    page.getByRole("button", { name: "Show Overview", exact: true }),
  ).toBeVisible();
  await main.screenshot({
    path: "test-results/pulse-prototype/project-detail-split.png",
  });
  await expect(main.getByTestId("pulse-workspace-projects")).toBeVisible();
  await expect(apps).toBeVisible();
  await page.goBack();
  await main.getByTestId("projects-section-projects").click();
  await expect(project).toBeVisible();
  await apps.getByRole("button", { name: "Messages", exact: true }).click();
  await expect(rail).toBeVisible();
  await rail.locator('[data-channel-name="bob-tyler"]').click();
  await expect(main.getByTestId("message-input")).toBeVisible();
  await expect(main.getByTestId("pulse-workspace-projects")).toHaveCount(0);
  const composer = main.getByTestId("message-input");
  await composer.fill("Keep this draft while switching apps");
  await apps.getByRole("button", { name: "Agents", exact: true }).click();
  await expect(rail).toHaveCount(0);
  await apps.getByRole("button", { name: "Messages", exact: true }).click();
  await expect(composer).toHaveText("Keep this draft while switching apps");
  await waitForAnimations(page);
  await page
    .getByTestId("unified-pulse")
    .screenshot({ path: "test-results/pulse-prototype/apps-messages.png" });
  await rail.getByRole("button", { name: "For you", exact: true }).click();
  await expect(main.getByTestId("pulse-briefing")).toBeVisible();
  await apps.getByRole("button", { name: "Projects", exact: true }).click();
  await main.getByTestId("projects-section-projects").click();
  await expect(project).toBeVisible();
  await waitForAnimations(page);
  await page.getByTestId("unified-pulse").screenshot({
    path: "test-results/pulse-prototype/workspace-projects.png",
  });
});

test("agents use human-style bubbles and shared typing feedback for typing and active turns", async ({
  page,
}) => {
  await seed(page);
  await page.clock.install();
  const rail = page.getByTestId("pulse-combined-list");
  const detail = page.getByTestId("pulse-combined-detail");
  await rail.locator('[data-channel-name="alice-tyler"]').click();
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "alice-tyler",
      kind: 20002,
    }),
  );
  await page.evaluate(
    ({ pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "alice-tyler",
        pubkey,
        content: "Here is the agent response",
        createdAt: Math.floor(Date.now() / 1000) - 10,
      }),
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );
  const row = detail
    .getByTestId("message-row")
    .filter({ hasText: "Here is the agent response" });
  await expect(row.getByTestId("message-body")).toHaveAttribute(
    "data-message-bubble",
    "incoming",
  );
  await expect(row).not.toContainText("owner unavailable");
  expect(
    await row
      .getByTestId("message-avatar")
      .evaluate(
        (avatar) =>
          Number.parseFloat(getComputedStyle(avatar).borderRadius) >=
          avatar.getBoundingClientRect().height / 2,
      ),
  ).toBe(true);
  await page.clock.fastForward(3_000);
  const indicator = detail
    .getByTestId("message-timeline")
    .getByTestId("timeline-typing-transition");
  const composer = detail.getByTestId("message-composer");
  const composerBefore = await boundsOf(composer);
  await page.evaluate(
    ({ pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
        channelName: "alice-tyler",
        pubkey,
      }),
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );
  await expect(indicator).toHaveAttribute("data-active", "true");
  await expect(indicator.locator(".typing-dot")).toHaveCount(3);
  await expect(detail.getByTestId("message-typing-avatar")).toHaveCount(1);
  await expect(row.getByTestId("message-avatar")).toHaveCount(0);
  await expect(detail.getByTestId("bot-activity-composer-trigger")).toHaveCount(
    0,
  );
  expect(await boundsOf(composer)).toEqual(composerBefore);
  await page.clock.fastForward(9_000);
  await expect(indicator).toHaveAttribute("data-active", "false");
  await expect(row.getByTestId("message-avatar")).toBeVisible();
  const channelId = await detail.getAttribute("data-channel-id");
  if (!channelId) throw new Error("Missing DM channel");
  // Observer-backed work must get the same feedback even without typing events.
  await page.evaluate(
    ({ agentPubkey, channelId }) =>
      window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey,
        channelId,
        turnId: "pulse-agent-response",
      }),
    { agentPubkey: TEST_IDENTITIES.alice.pubkey, channelId },
  );
  await expect(indicator).toHaveAttribute("data-active", "true");
  await expect(detail.getByTestId("bot-activity-composer-trigger")).toHaveCount(
    0,
  );
  await waitForAnimations(page);
  await detail.screenshot({
    path: "test-results/pulse-prototype/agent-typing.png",
  });
  await page.evaluate(
    ({ agentPubkey, channelId }) =>
      window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
        agentPubkey,
        channelId,
        turnId: "pulse-agent-response",
        kind: "turn_completed",
      }),
    { agentPubkey: TEST_IDENTITIES.alice.pubkey, channelId },
  );
  await expect(indicator).toHaveAttribute("data-active", "false");
  await rail.locator('[data-channel-name="random"]').click();
  await detail
    .getByTestId("message-row")
    .filter({ hasText: "What if catching up" })
    .getByTestId("message-thread-summary")
    .click();
  const body = detail.getByTestId("message-thread-body");
  await expect(body).toBeVisible();
  await waitForAnimations(page);
  await page.clock.fastForward(3_000);
  const threadHeadId = new URLSearchParams(page.url().split("?")[1]).get(
    "thread",
  );
  if (!threadHeadId) throw new Error("Missing thread");
  await page.evaluate(
    ({ agent, human, threadHeadId }) => {
      for (const pubkey of [agent, human])
        window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
          channelName: "random",
          pubkey,
          threadHeadId,
        });
    },
    {
      agent: TEST_IDENTITIES.alice.pubkey,
      human: TEST_IDENTITIES.bob.pubkey,
      threadHeadId,
    },
  );
  await expect(body.getByTestId("timeline-typing-transition")).toHaveAttribute(
    "data-active",
    "true",
  );
  await expect(body.getByTestId("message-typing-avatar")).toHaveCount(2);
  await expect(body.locator(".typing-dots-bubble")).toHaveCount(1);
  await expect(detail.getByTestId("bot-activity-composer-trigger")).toHaveCount(
    0,
  );
  await page
    .getByRole("button", { name: "Back to conversation", exact: true })
    .click();
  await expect(
    detail
      .getByTestId("message-timeline")
      .getByTestId("timeline-typing-transition"),
  ).toHaveAttribute("data-active", "false");
});

test("agent conversation rows replace unread dots with notification-sized working dots", async ({
  page,
}) => {
  await seed(page);
  const rail = page.getByTestId("pulse-combined-list");
  const agentRow = rail.locator('[data-channel-name="alice-tyler"]');
  const humanRow = rail.locator('[data-channel-name="bob-tyler"]');
  const detail = page.getByTestId("pulse-combined-detail");
  await agentRow.click();
  await expect(detail.getByTestId("message-input")).toBeVisible();
  const channelId = await detail.getAttribute("data-channel-id");
  if (!channelId) throw new Error("Missing agent conversation");
  await expect(agentRow.getByTestId("pulse-unread-dot")).toHaveCount(0);
  await rail.getByRole("button", { name: "For you", exact: true }).click();
  await page.evaluate(
    ({ pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "alice-tyler",
        pubkey,
        content: "Unread agent reply",
        createdAt: Math.floor(Date.now() / 1000) + 1,
      }),
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );
  const unread = agentRow.getByTestId("pulse-unread-dot");
  await expect(unread).toBeVisible();
  const size = await unread.evaluate((dot) => ({
    width: getComputedStyle(dot).width,
    height: getComputedStyle(dot).height,
    color: getComputedStyle(dot).backgroundColor,
  }));
  let turnNumber = 0;
  const setWorking = async (kind: "turn_started" | "turn_completed") => {
    if (kind === "turn_started") turnNumber += 1;
    await page.evaluate(
      ({ agentPubkey, channelId, kind, turnNumber }) =>
        window.__BUZZ_E2E_SEED_ACTIVE_TURNS__?.({
          agentPubkey,
          channelId,
          turnId: `sidebar-working-${turnNumber}`,
          kind,
        }),
      {
        agentPubkey: TEST_IDENTITIES.alice.pubkey,
        channelId,
        kind,
        turnNumber,
      },
    );
  };
  await setWorking("turn_started");
  const working = agentRow.getByTestId("pulse-working-dots");
  await expect(working).toBeVisible();
  await expect(unread).toHaveCount(0);
  await expect(agentRow).toHaveAttribute(
    "aria-description",
    "Agent working. Unread messages",
  );
  const dots = working.locator(".typing-dot");
  await expect(dots).toHaveCount(3);
  for (const dot of await dots.all()) {
    await expect(dot).toHaveCSS("width", size.width);
    await expect(dot).toHaveCSS("height", size.height);
    await expect(dot).toHaveCSS(
      "background-color",
      await themeColor(page, "--muted-foreground"),
    );
    await expect(dot).toHaveCSS("animation-name", "typing-dot-pulse");
  }
  const rowBounds = await boundsOf(agentRow);
  const indicatorBounds = await boundsOf(working);
  expect(
    rowBounds.x + rowBounds.width - indicatorBounds.x - indicatorBounds.width,
  ).toBeCloseTo(12, 0);
  await expect(humanRow.getByTestId("pulse-working-dots")).toHaveCount(0);
  await page
    .getByRole("button", { name: "App variations", exact: true })
    .click();
  await page
    .getByRole("menuitemradio", { name: "Separate feeds", exact: true })
    .click();
  await expect(working).toBeVisible();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(dots.first()).toHaveCSS("animation-name", "none");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await waitForAnimations(page);
  await rail.screenshot({
    path: "test-results/pulse-prototype/rail-agent-working.png",
  });
  await setWorking("turn_completed");
  await expect(working).toHaveCount(0);
  await expect(unread).toBeVisible();
  await agentRow.click();
  await expect(unread).toHaveCount(0);
  await setWorking("turn_started");
  await expect(working).toBeVisible();
  await setWorking("turn_completed");
  await expect(working).toHaveCount(0);
  await expect(unread).toHaveCount(0);
});

test("workspace profile panels share the main container's frame and remain resizable", async ({
  page,
}) => {
  await seed(page);
  await page
    .getByTestId("pulse-app-navigation")
    .getByRole("button", { name: "Projects", exact: true })
    .click();
  // Exercise a profile deep link through the real Pulse route and profile panel.
  await page.evaluate((pubkey) => {
    const [path, query] = location.hash.split("?");
    const params = new URLSearchParams(query);
    params.set("profile", pubkey);
    location.hash = `${path}?${params}`;
  }, TEST_IDENTITIES.bob.pubkey);
  const panel = page.getByTestId("user-profile-panel");
  const main = page.getByTestId("pulse-main-container");
  await expect(panel).toBeVisible();
  await expect(panel).toHaveCSS("border-radius", "24px");
  await expect(panel).toHaveCSS("border-width", "0px");
  await expect(main).toHaveAttribute("data-expanded", "true");
  await waitForAnimations(page);
  const panelBox = await boundsOf(panel);
  const mainBox = await boundsOf(main);
  expect(panelBox.y).toBe(mainBox.y);
  expect(panelBox.height).toBe(mainBox.height);
  expect(panelBox.x - mainBox.x - mainBox.width).toBeCloseTo(8, 0);
  expect(panelBox.x + panelBox.width).toBe(1432);
  await page.screenshot({
    path: "test-results/pulse-prototype/workspace-profile-frame.png",
  });
  const handle = await boundsOf(
    panel.getByTestId("user-profile-resize-handle"),
  );
  // Use the part of the resize target inside the rounded surface.
  const x = panelBox.x + 2;
  const y = handle.y + handle.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 40, y, { steps: 5 });
  await page.mouse.up();
  await expect
    .poll(async () => (await boundsOf(panel)).width)
    .toBeGreaterThan(panelBox.width + 20);
  await panel.getByRole("button", { name: "Close panel", exact: true }).click();
  await expect(panel).toHaveCount(0);
  await expect(main).toHaveCSS("max-width", "960px");
  const restored = await boundsOf(main);
  expect(restored.x + restored.width / 2).toBeCloseTo(720, 0);
});
