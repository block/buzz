import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Guards the NIP-IA discovery-suppression contract (Dawn's table v2):
// - Members sidebar: archived members fold under "Archived (N)", not in the
//   active people/bots lists.
// - Mention autocomplete: archived members are filtered out of suggestions
//   (but their `members` entry still resolves historical @-mentions).
// - DM picker: archived users are omitted from search results.
// - History invariant: an archived user's existing channel message renders
//   normally (Tyler's "never hide messages").

const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";
// Active mock identity (matches DEFAULT_MOCK_IDENTITY.pubkey in e2eBridge).
// Used to exercise the self-exemption rule in the predicate.
const SELF_PUBKEY = "deadbeef".repeat(8);

test.describe("NIP-IA hide archived from discovery", () => {
  test("members sidebar: archived member folds under Archived section", async ({
    page,
  }) => {
    await installMockBridge(page, { archivedIdentities: [ALICE_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();

    // Active People list should NOT include Alice.
    const peopleList = page.getByTestId("members-sidebar-people");
    await expect(peopleList).not.toContainText("alice");

    // Folded Archived section is visible with count = 1.
    await expect(page.getByTestId("members-sidebar-archived")).toBeVisible();
    await expect(page.getByTestId("members-sidebar-archived-count")).toHaveText(
      "(1)",
    );

    // Expanding it reveals Alice.
    await page.getByTestId("members-sidebar-archived").click();
    await expect(
      page.getByTestId("members-sidebar-archived-list"),
    ).toContainText("alice");
  });

  test("members sidebar: no Archived section when no archived members", async ({
    page,
  }) => {
    await installMockBridge(page, { archivedIdentities: [] });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();
    await expect(page.getByTestId("members-sidebar-archived")).toHaveCount(0);
  });

  test("mention autocomplete: archived member is filtered out", async ({
    page,
  }) => {
    await installMockBridge(page, { archivedIdentities: [ALICE_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Type `@a` to trigger autocomplete. Without filtering Alice would match.
    const input = page.getByTestId("message-input");
    await input.click();
    await input.pressSequentially("@a");

    // Alice's suggestion testid does NOT appear.
    await expect(
      page.getByTestId(`mention-suggestion-${ALICE_PUBKEY}`),
    ).toHaveCount(0);

    // Sanity: Bob (also `@b...` candidate) is available when his query starts;
    // proves the autocomplete itself is functional, not just always-empty.
    await input.fill("@b");
    await expect(
      page.getByTestId(`mention-suggestion-${BOB_PUBKEY}`),
    ).toBeVisible();
  });

  test("DM picker: archived user is omitted from search results", async ({
    page,
  }) => {
    await installMockBridge(page, { archivedIdentities: [ALICE_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("new-dm-trigger").click();
    await expect(page.getByTestId("new-dm-dialog")).toBeVisible();

    await page.getByTestId("new-dm-search").fill("alice");
    // Alice's result row does NOT appear, even though search would normally
    // surface her by display name.
    await expect(page.getByTestId(`new-dm-result-${ALICE_PUBKEY}`)).toHaveCount(
      0,
    );

    // Sanity: Bob is still searchable, confirming the dialog works.
    await page.getByTestId("new-dm-search").fill("bob");
    await expect(page.getByTestId(`new-dm-result-${BOB_PUBKEY}`)).toBeVisible();
  });

  test("history invariant: archived user's existing message still renders with their name", async ({
    page,
  }) => {
    // Alice has a seeded message in #general from the prior PR's e2e setup
    // (see e2eBridge.ts seed). Archiving her must not remove that message.
    await installMockBridge(page, { archivedIdentities: [ALICE_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Alice's seed message renders with her display name in the timeline.
    const aliceMessage = page.getByTestId("message-row").nth(1);
    await expect(aliceMessage).toContainText("alice");
    await expect(aliceMessage).toContainText("Hey team — checking in.");
  });

  test("self-exemption: archived current user still appears in their own People list (not folded)", async ({
    page,
  }) => {
    // Anti-shadowban property: the current user is never hidden from their
    // own client even when archived on the relay. The predicate's
    // self-exemption is what enforces this — without it, the user would lose
    // their own seat in the members sidebar.
    await installMockBridge(page, { archivedIdentities: [SELF_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();

    // Self appears in active People list — NOT folded into Archived.
    const peopleList = page.getByTestId("members-sidebar-people");
    await expect(peopleList).toContainText("You");
    await expect(
      peopleList.getByTestId(`sidebar-member-${SELF_PUBKEY}`),
    ).toBeVisible();
    // No Archived section at all when self is the only archived pubkey in
    // the channel (self-exemption makes archived.length === 0).
    await expect(page.getByTestId("members-sidebar-archived")).toHaveCount(0);
  });

  test("self-exemption: archived current user can still self-mention in own autocomplete", async ({
    page,
  }) => {
    // Self-mention is a no-op in practice, but the predicate must not filter
    // self from their own autocomplete — that would be the shadowban NIP-IA
    // exists to prevent.
    await installMockBridge(page, { archivedIdentities: [SELF_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const input = page.getByTestId("message-input");
    await input.click();
    await input.pressSequentially("@npub");
    // Self's suggestion appears in autocomplete despite being in the archived
    // set — the predicate's self-exemption fires.
    await expect(
      page.getByTestId(`mention-suggestion-${SELF_PUBKEY}`),
    ).toBeVisible();
  });

  // Member-adder (ChannelMemberInviteCard) — the invite search excludes
  // existing channel members, so we test in #agents where Alice is NOT a
  // member (only the mock identity + Charlie are): the ONLY thing that can
  // drop her from results is the archive filter. The control run (nothing
  // archived) proves she would otherwise appear — guarding a vacuous green.
  test("member-adder: archived non-member is omitted from invite search", async ({
    page,
  }) => {
    await installMockBridge(page, { archivedIdentities: [ALICE_PUBKEY] });
    await page.goto("/");
    await page.getByTestId("channel-agents").click();
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();
    await page.getByTestId("channel-management-search-users").fill("alice");
    await expect(
      page.getByTestId(`channel-user-search-result-${ALICE_PUBKEY}`),
    ).toHaveCount(0);
  });

  test("member-adder: control — non-archived non-member IS in invite search", async ({
    page,
  }) => {
    // Same channel/query, nothing archived: Alice now appears. This is the
    // companion that makes the test above non-vacuous (proves she was dropped
    // by the archive filter, not by member-exclusion or a broken search).
    await installMockBridge(page, { archivedIdentities: [] });
    await page.goto("/");
    await page.getByTestId("channel-agents").click();
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();
    await page.getByTestId("channel-management-search-users").fill("alice");
    await expect(
      page.getByTestId(`channel-user-search-result-${ALICE_PUBKEY}`),
    ).toBeVisible();
  });
});
