import { expect, test, type Page } from "@playwright/test";

import {
  installMockBridge,
  openChannelBrowser,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const MOCK_PUBKEY = "deadbeef".repeat(8);
const CUSTOM_SECTION = { id: "sec-projects", name: "Projects", order: 0 };

async function seedCustomSection(page: Page) {
  await page.addInitScript(
    ({ pubkey, section }) => {
      window.localStorage.setItem(
        `buzz-channel-sections.v1:${pubkey}`,
        JSON.stringify({ version: 1, sections: [section], assignments: {} }),
      );
    },
    { pubkey: MOCK_PUBKEY, section: CUSTOM_SECTION },
  );
}

test.beforeEach(async ({ page }, testInfo) => {
  await installMockBridge(
    page,
    testInfo.title.includes("failed section create")
      ? { createChannelErrors: ["Create failed"] }
      : testInfo.title.includes("pre-join roster")
        ? {
            // The strongest-authority pre-join viewer: a community admin who
            // also owns a managed bot inside the unjoined channel. Neither
            // authority may surface row actions before Join. Membership
            // support must be on for the NIP-43 snapshot (and thus the relay
            // role) to reach the client at all.
            relayRequiresMembership: true,
            relayRole: "admin",
            managedAgents: [
              {
                pubkey: TEST_IDENTITIES.charlie.pubkey,
                name: "scout",
                status: "running",
                channelNames: ["design"],
              },
            ],
          }
        : undefined,
  );
});

test("keyboard shortcut opens the channel browser dialog", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  const isMacBrowser = await page.evaluate(() =>
    /mac|iphone|ipad|ipod/i.test(navigator.platform),
  );

  if (isMacBrowser) {
    await page.evaluate(() => {
      window.dispatchEvent(
        new KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          key: "O",
          metaKey: true,
          shiftKey: true,
        }),
      );
    });
  } else {
    await page.keyboard.press("Control+Shift+O");
  }
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
});

test("channel browser shows channels not yet joined", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Browse channels" }),
  ).toBeVisible();

  // "design" and "sales" are open channels the mock user is NOT a member of
  await expect(page.getByTestId("browse-channel-design")).toBeVisible();
  await expect(page.getByTestId("browse-channel-sales")).toBeVisible();

  // "general" is a channel the mock user IS a member of — shown in "Joined" section
  await expect(page.getByTestId("browse-channel-general")).toBeVisible();
});

test("channel browser sorts alphabetically or by member count", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  const rows = page.locator('[data-testid^="browse-channel-"]');

  await expect(rows).toHaveText([
    /#agents/,
    /#all-replies/,
    /#buzz/,
    /#deep-history/,
    /#design/,
    /#engineering/,
    /#general/,
    /#random/,
    /#sales/,
    /#secret-projects/,
    /#welcome-everyone/,
  ]);

  await page.getByTestId("channel-browser-sort").click();
  await page.getByTestId("channel-browser-sort-members").click();

  await expect(rows).toHaveText([
    /#general/,
    /#agents/,
    /#engineering/,
    /#random/,
    /#all-replies/,
    /#deep-history/,
    /#design/,
    /#sales/,
    /#secret-projects/,
    /#buzz/,
    /#welcome-everyone/,
  ]);
  await expect(page.getByTestId("channel-browser-sort")).toHaveAttribute(
    "aria-label",
    "Sort channels: Most members",
  );
});

test("channel browser sorts by recent activity", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  const rows = page.locator('[data-testid^="browse-channel-"]');

  await page.getByTestId("channel-browser-sort").click();
  await page.getByTestId("channel-browser-sort-recent").click();

  await expect(rows).toHaveText([
    /#all-replies/,
    /#deep-history/,
    /#general/,
    /#agents/,
    /#sales/,
    /#engineering/,
    /#design/,
    /#buzz/,
    /#random/,
    /#secret-projects/,
    /#welcome-everyone/,
  ]);
  await expect(page.getByTestId("channel-browser-sort")).toHaveAttribute(
    "aria-label",
    "Sort channels: Recent",
  );
});

test("channel browser search filters by name", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  await page.getByTestId("channel-browser-search").fill("design");

  await expect(page.getByTestId("browse-channel-design")).toBeVisible();
  await expect(page.getByTestId("browse-channel-sales")).toHaveCount(0);
  await expect(page.getByTestId("browse-channel-general")).toHaveCount(0);
});

test("channel browser search filters by description", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("pipeline");

  // "sales" has "pipeline" in its description
  await expect(page.getByTestId("browse-channel-sales")).toBeVisible();
  await expect(page.getByTestId("browse-channel-design")).toHaveCount(0);
});

test("channel browser shows no results for unmatched search", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("zzz-nonexistent");

  await expect(page.getByText("No channels match your search")).toBeVisible();
});

test("channel browser fuzzy-matches a subsequence", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  // "engr" is not a substring of "engineering", but it is an in-order
  // subsequence — plain includes() would miss it, fuzzy matching finds it.
  await page.getByTestId("channel-browser-search").fill("engr");

  await expect(page.getByTestId("browse-channel-engineering")).toBeVisible();
  await expect(page.getByTestId("browse-channel-general")).toHaveCount(0);
});

test("channel browser matches a scattered subsequence", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  // "sls" is neither a substring nor a prefix of "sales" — it only matches as
  // an in-order subsequence (s·a·l·e·s). Proves fuzzy matching end-to-end.
  await page.getByTestId("channel-browser-search").fill("sls");

  await expect(page.getByTestId("browse-channel-sales")).toBeVisible();
  await expect(page.getByTestId("browse-channel-general")).toHaveCount(0);
});

test("channel browser ranks the best match first", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  // "gen" is a prefix of "general" (strong match) but only a substring of
  // "agents" and a subsequence of "engineering" (weaker). The prefix match
  // should float to the top regardless of the alphabetical default sort.
  await page.getByTestId("channel-browser-search").fill("gen");

  const firstRow = page.getByTestId(/^browse-channel-/).first();
  await expect(firstRow).toHaveAttribute(
    "data-testid",
    "browse-channel-general",
  );
});

test("sidebar add-channel button creates without treating the click as a callback", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  await page.getByTestId("section-actions-channels-quick-create").click();

  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  const channelName = `sidebar-created-${Date.now()}`;
  await page.getByTestId("channel-browser-search").fill(channelName);
  await page.getByTestId("channel-browser-create-row").click();
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("stream-list")).toContainText(channelName);
});

test("custom section add button creates directly into that section", async ({
  page,
}) => {
  await seedCustomSection(page);
  await page.goto("/");

  const addButton = page.getByTestId(
    `section-actions-${CUSTOM_SECTION.id}-quick-create`,
  );
  await expect(addButton).toHaveAccessibleName("Add channel to Projects");
  await addButton.click();
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  const channelName = `section-created-${Date.now()}`;
  await page.getByTestId("channel-browser-search").fill(channelName);
  await page.getByTestId("channel-browser-create-row").click();
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(
    page.getByTestId(`section-title-${CUSTOM_SECTION.id}`),
  ).toBeVisible();
  await expect(page.getByTestId(`channel-${channelName}`)).toBeVisible();
  await expect(page.getByTestId("stream-list")).not.toContainText(channelName);
});

test("canceling section create does not affect the next global create", async ({
  page,
}) => {
  await seedCustomSection(page);
  await page.goto("/");

  await page
    .getByTestId(`section-actions-${CUSTOM_SECTION.id}-quick-create`)
    .click();
  // Gate on the dialog mounting before dismissing it. Escape sent before mount
  // is dropped (no handler yet), and not.toBeVisible() then passes vacuously
  // against a dialog that hasn't rendered — so the dialog opens *after* the
  // assertion and its overlay swallows every later click.
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  // The overlay outlives the content by one exit animation; wait for it to
  // detach so it can't intercept the next click.
  await expect(page.getByTestId("dialog-overlay")).toHaveCount(0);

  await page.getByTestId("section-actions-channels-quick-create").click();
  const channelName = `global-after-cancel-${Date.now()}`;
  await page.getByTestId("channel-browser-search").fill(channelName);
  await page.getByTestId("channel-browser-create-row").click();
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
});

test("failed section create retry still assigns to the section", async ({
  page,
}) => {
  await seedCustomSection(page);
  await page.goto("/");

  await page
    .getByTestId(`section-actions-${CUSTOM_SECTION.id}-quick-create`)
    .click();
  const channelName = `section-retry-${Date.now()}`;
  await page.getByTestId("channel-browser-search").fill(channelName);
  await page.getByTestId("channel-browser-create-row").click();
  await page.getByTestId("create-channel-submit").click();
  await expect(page.getByText("Create failed")).toBeVisible();

  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId(`channel-${channelName}`)).toBeVisible();
  await expect(page.getByTestId("stream-list")).not.toContainText(channelName);
});

test("create affordance is visible on open before typing", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);

  // The create row is present from the get-go so it's clear you can browse OR
  // create — not just after you start typing.
  const createRow = page.getByTestId("channel-browser-create-row");
  await expect(createRow).toBeVisible();
  await expect(createRow).toContainText("Create a new channel");
});

test("typing a partial match surfaces a persistent create row", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  // "desig" matches "design" by substring but is not an exact channel name,
  // so both the matching channel AND the create row are shown.
  await page.getByTestId("channel-browser-search").fill("desig");

  const createRow = page.getByTestId("channel-browser-create-row");
  await expect(createRow).toBeVisible();
  await expect(createRow).toContainText("desig");
  await expect(page.getByTestId("browse-channel-design")).toBeVisible();
});

test("exact name match hides the create row", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("general");

  await expect(page.getByTestId("browse-channel-general")).toBeVisible();
  await expect(page.getByTestId("channel-browser-create-row")).toHaveCount(0);
});

test("no-match search pins a create row above the empty state", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("zzz-nonexistent");

  await expect(page.getByText("No channels match your search")).toBeVisible();
  const createRow = page.getByTestId("channel-browser-create-row");
  await expect(createRow).toBeVisible();
  await expect(createRow).toContainText("zzz-nonexistent");
});

test("create row leads to the prefilled create form", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill("desig");
  await page.getByTestId("channel-browser-create-row").click();

  // Create mode reuses the shared form; the name is prefilled from the query.
  await expect(page.getByTestId("create-channel-name")).toHaveValue("desig");

  // Back returns to the search list without closing the dialog.
  await page.getByTestId("channel-browser-create-back").click();
  await expect(page.getByTestId("channel-browser-search")).toBeVisible();
});

test("creating from the browser adds the channel to the sidebar", async ({
  page,
}) => {
  const channelName = `browse-created-${Date.now()}`;

  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill(channelName);
  await page.getByTestId("channel-browser-create-row").click();

  await expect(page.getByTestId("create-channel-name")).toHaveValue(
    channelName,
  );
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toContainText(channelName);
});

test("Enter with no matches jumps to create", async ({ page }) => {
  const channelName = `enter-created-${Date.now()}`;

  await page.goto("/");

  await openChannelBrowser(page);
  await page.getByTestId("channel-browser-search").fill(channelName);
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("create-channel-name")).toHaveValue(
    channelName,
  );
});

test("arrow keys reach the pinned create row and Enter activates it", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  // "desig" keeps a channel match (#design) AND the create row visible, so the
  // create row is not the only actionable item — it must be reachable by
  // keyboard, not just Tab.
  await page.getByTestId("channel-browser-search").fill("desig");

  const createRow = page.getByTestId("channel-browser-create-row");
  await expect(createRow).toBeVisible();

  // The create row is pinned at the top → first ArrowDown highlights it.
  await page.keyboard.press("ArrowDown");
  await expect(createRow).toHaveAttribute("data-selected", "true");

  // Enter on the highlighted create row enters the prefilled create form.
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("create-channel-name")).toHaveValue("desig");
});

test("Enter selects a channel when create row is not highlighted", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  // With the create row present but NOT highlighted, Enter should still select
  // the first channel match rather than jumping to create.
  await page.getByTestId("channel-browser-search").fill("desig");
  await expect(page.getByTestId("browse-channel-design")).toBeVisible();

  await page.keyboard.press("Enter");

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText("design");
});

test("non-member open channel header shows its member count and roster", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  const designRow = page.getByTestId("browse-channel-design");
  await expect(designRow).toContainText("2 members");
  await designRow.locator("button").first().click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText("design");
  await expect(
    page.getByRole("button", { name: "Join", exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("channel-members-trigger")).toHaveAttribute(
    "aria-label",
    "View channel members (2)",
  );
  await expect(page.getByTestId("channel-start-huddle-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-management-trigger")).toHaveCount(0);

  await page.getByTestId("channel-members-trigger").click();
  const members = page.getByTestId("members-sidebar");
  await expect(members).toBeVisible();
  await expect(
    members.getByTestId(
      "sidebar-member-953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
    ),
  ).toBeVisible();
  await expect(
    members.getByTestId(
      "sidebar-member-bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
    ),
  ).toBeVisible();

  const memberSearch = members.getByTestId("channel-management-search-users");
  await expect(memberSearch).toHaveAttribute(
    "placeholder",
    "Search people and agents",
  );
  await memberSearch.fill("char");
  await expect(
    members.getByText("Not in this channel", { exact: true }),
  ).toHaveCount(0);
  await expect(
    members.locator('[data-testid^="channel-user-search-result-"]'),
  ).toHaveCount(0);
});

async function openMemberMenu(page: Page, pubkey: string) {
  const row = page.getByTestId(`sidebar-member-${pubkey}`);
  const trigger = page.getByTestId(`sidebar-member-menu-${pubkey}`);
  await row.scrollIntoViewIfNeeded();
  await row.hover();
  // Radix dropdown re-opens are unreliable via pointer after a prior item
  // interaction; keyboard open (focus + Enter) is deterministic.
  await trigger.focus();
  await trigger.press("Enter");
  await expect(trigger).toHaveAttribute("data-state", "open");
}

test("pre-join roster is read-only until the viewer joins", async ({
  page,
}) => {
  // Seeded by beforeEach: the viewer is a community admin (relay role) and
  // owns the running managed bot "scout" that is a member of #design.
  const botPubkey = TEST_IDENTITIES.charlie.pubkey;
  const bobPubkey = TEST_IDENTITIES.bob.pubkey;

  await page.goto("/");
  await openChannelBrowser(page);
  await page
    .getByTestId("browse-channel-design")
    .locator("button")
    .first()
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("design");
  const joinButton = page.getByRole("button", { name: "Join", exact: true });
  await expect(joinButton).toBeVisible();
  await expect(page.getByTestId("channel-members-trigger")).toHaveAttribute(
    "aria-label",
    "View channel members (3)",
  );

  await page.getByTestId("channel-members-trigger").click();
  const members = page.getByTestId("members-sidebar");
  await expect(members).toBeVisible();

  // The roster reads normally — human rows and the owned bot's status badge —
  await expect(
    members.getByTestId(`sidebar-member-${bobPubkey}`),
  ).toBeVisible();
  await expect(
    members.getByTestId(`sidebar-member-${botPubkey}`),
  ).toBeVisible();
  await expect(
    members.getByTestId(`sidebar-managed-agent-status-${botPubkey}`),
  ).toBeVisible();

  // — but neither community-admin moderation nor owned-agent lifecycle may
  // surface any row menu or bulk agent control before Join.
  await expect(
    members.locator('[data-testid^="sidebar-member-menu-"]'),
  ).toHaveCount(0);
  await expect(
    members.getByTestId("members-sidebar-agent-controls"),
  ).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(members).not.toBeVisible();

  await joinButton.click();
  await expect(joinButton).toHaveCount(0);
  await expect(page.getByTestId("channel-members-trigger")).toHaveAttribute(
    "aria-label",
    "View channel members (4)",
  );

  // The same authorities light up once the viewer is a member: relay-admin
  // moderation on a human row, lifecycle + access management on the owned bot.
  await page.getByTestId("channel-members-trigger").click();
  await expect(members).toBeVisible();

  await openMemberMenu(page, bobPubkey);
  await expect(page.getByTestId(`sidebar-ban-${bobPubkey}`)).toBeVisible();
  await page.keyboard.press("Escape");

  await openMemberMenu(page, botPubkey);
  await expect(
    page.getByTestId(`sidebar-agent-action-${botPubkey}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`sidebar-edit-respond-to-${botPubkey}`),
  ).toBeVisible();
});

test("joining a channel from browser adds it to the sidebar", async ({
  page,
}) => {
  await page.goto("/");

  // Verify "design" is not in the sidebar
  const streamList = page.getByTestId("stream-list");
  await expect(streamList).not.toContainText("design");

  // Open browser and join
  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page
    .getByTestId("browse-channel-design")
    .getByRole("button", { name: "Join" })
    .click();

  // Dialog should close and navigate to the joined channel
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page).toHaveURL(/#\/channels\//);
  await expect(page.getByTestId("chat-title")).toHaveText("design");

  // Channel should now appear in the sidebar
  await expect(streamList).toContainText("design");
});

test("clicking a joined channel in browser navigates to it", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // "general" is already joined — clicking should navigate without join
  await page.getByTestId("browse-channel-general").click();

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page).toHaveURL(/#\/channels\//);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("channel browser does not show DM or private channels", async ({
  page,
}) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // DM channels should not appear
  await expect(page.getByTestId("browse-channel-alice-tyler")).toHaveCount(0);
  await expect(page.getByTestId("browse-channel-bob-tyler")).toHaveCount(0);

  // Private forum should not appear
  await expect(page.getByTestId("browse-channel-announcements")).toHaveCount(0);
});

test("channel browser closes on escape", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
});

test("keyboard navigation works in channel browser", async ({ page }) => {
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();

  // Filter to unjoined channels only to get a predictable list
  await page.getByTestId("channel-browser-search").fill("design");
  await expect(page.getByTestId("browse-channel-design")).toBeVisible();

  // Press Enter to join the selected (first) channel
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("channel-browser-dialog")).not.toBeVisible();
  await expect(page.getByTestId("chat-title")).toHaveText("design");
});

test("sidebar only shows channels the user has joined", async ({ page }) => {
  await page.goto("/");

  const streamList = page.getByTestId("stream-list");

  // Channels the mock user IS a member of
  await expect(streamList).toContainText("general");
  await expect(streamList).toContainText("random");
  await expect(streamList).toContainText("engineering");
  await expect(streamList).toContainText("agents");

  // Channels the mock user is NOT a member of
  await expect(streamList).not.toContainText("design");
  await expect(streamList).not.toContainText("sales");
});
