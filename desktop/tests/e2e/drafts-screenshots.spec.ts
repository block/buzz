import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/drafts";

// Mock bridge default pubkey — must match DEFAULT_MOCK_PUBKEY in bridge.ts
const MOCK_PUBKEY = "deadbeef".repeat(8);
const DRAFT_STORE_KEY = `buzz-drafts.v1:${MOCK_PUBKEY}`;

// Channel IDs from the mock bridge seed data
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENTS_CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";

// Fixed timestamps for deterministic rendering
const CREATED_AT_1 = "2026-07-01T10:00:00.000Z";
const CREATED_AT_2 = "2026-07-02T14:30:00.000Z";
const CREATED_AT_3 = "2026-07-03T09:15:00.000Z";
const CREATED_AT_SENT = "2026-07-04T16:45:00.000Z";

type StoredDraftState = {
  content: string;
  selectionStart: number;
  selectionEnd: number;
  channelId: string;
  createdAt: string;
  updatedAt: string;
  pendingImeta: unknown[];
  spoileredAttachmentUrls: string[];
  status: "active" | "sent";
};

type StoredDrafts = Record<string, StoredDraftState>;

/** Active drafts: text draft in #general, image-only draft in #agents, long text draft. */
const ACTIVE_DRAFTS: StoredDrafts = {
  [`channel:${GENERAL_CHANNEL_ID}`]: {
    content:
      "Hey team — I've been working on the new onboarding flow. Check out the latest mockups when you get a chance!",
    selectionStart: 107,
    selectionEnd: 107,
    channelId: GENERAL_CHANNEL_ID,
    createdAt: CREATED_AT_1,
    updatedAt: CREATED_AT_1,
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    status: "active",
  },
  [`channel:${AGENTS_CHANNEL_ID}`]: {
    // Image-only draft — exercises the "1 attachment" fallback in getDraftPreview
    content: "",
    selectionStart: 0,
    selectionEnd: 0,
    channelId: AGENTS_CHANNEL_ID,
    createdAt: CREATED_AT_2,
    updatedAt: CREATED_AT_2,
    pendingImeta: [
      {
        url: "https://example.com/screenshot.png",
        sha256: "abc123",
        size: 204800,
        type: "image/png",
        dim: "1280x900",
      },
    ],
    spoileredAttachmentUrls: [],
    status: "active",
  },
};

/** Active drafts + one sent record for shots that need both subsections. */
const ACTIVE_AND_SENT_DRAFTS: StoredDrafts = {
  ...ACTIVE_DRAFTS,
  [`sent:channel:${GENERAL_CHANNEL_ID}:1720115100000-1`]: {
    content:
      "Shipping the draft message improvements in PR #1539 — image persistence, sent records, and the new Drafts inbox section.",
    selectionStart: 119,
    selectionEnd: 119,
    channelId: GENERAL_CHANNEL_ID,
    createdAt: CREATED_AT_3,
    updatedAt: CREATED_AT_SENT,
    pendingImeta: [],
    spoileredAttachmentUrls: [],
    status: "sent",
  },
};

/**
 * Patch the mock workspace to include the pubkey so initDraftStore gets the
 * correct pubkey on app startup. The workspace is seeded by installMockBridge
 * without a pubkey field; this addInitScript runs after that seed (init
 * scripts execute in registration order) and adds it.
 */
async function patchWorkspacePubkey(page: import("@playwright/test").Page) {
  await page.addInitScript(
    ({ pubkey }) => {
      const raw = window.localStorage.getItem("buzz-workspaces");
      const workspaces = raw
        ? (JSON.parse(raw) as Array<Record<string, unknown>>)
        : [];
      if (workspaces[0]) {
        workspaces[0].pubkey = pubkey;
        window.localStorage.setItem(
          "buzz-workspaces",
          JSON.stringify(workspaces),
        );
      }
    },
    { pubkey: MOCK_PUBKEY },
  );
}

/** Seed draft localStorage before page load via addInitScript. */
async function seedDraftStore(
  page: import("@playwright/test").Page,
  drafts: StoredDrafts,
) {
  await page.addInitScript(
    ({ storeKey, value }) => {
      window.localStorage.setItem(storeKey, JSON.stringify(value));
    },
    { storeKey: DRAFT_STORE_KEY, value: drafts },
  );
}

/** Navigate to `/`, wait for inbox, then select the Drafts filter. */
async function openDraftsPanel(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("home-inbox")).toBeVisible({ timeout: 10_000 });

  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Drafts" }).click();

  // Dismiss the dropdown so it doesn't obscure the panel assertions.
  await page.keyboard.press("Escape");

  const panel = page.getByTestId("home-inbox-drafts");
  await expect(panel).toBeVisible({ timeout: 8_000 });
  return panel;
}

test.describe("drafts screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 500));
      }
    });
  });

  test("01 — drafts section populated", async ({ page }) => {
    await installMockBridge(page);
    await patchWorkspacePubkey(page);
    await seedDraftStore(page, ACTIVE_DRAFTS);

    const panel = await openDraftsPanel(page);

    // Both active draft rows should be visible
    const draftRows = panel.locator("[data-testid^='home-draft-item-']");
    await expect(draftRows).toHaveCount(2, { timeout: 6_000 });

    // The text draft row shows content
    await expect(
      panel.getByText(
        "Hey team — I've been working on the new onboarding flow.",
      ),
    ).toBeVisible({ timeout: 5_000 });

    // The image-only draft shows the attachment fallback
    await expect(panel.getByText("1 attachment")).toBeVisible({
      timeout: 5_000,
    });

    // Section heading should be "DRAFTS"
    await expect(panel.getByText("Drafts", { exact: true })).toBeVisible();

    // Small settle before screenshot
    await page.waitForTimeout(200);

    await panel.screenshot({
      path: `${SHOTS}/01-drafts-section-populated.png`,
    });
  });

  test("02 — sent subsection visible", async ({ page }) => {
    await installMockBridge(page);
    await patchWorkspacePubkey(page);
    await seedDraftStore(page, ACTIVE_AND_SENT_DRAFTS);

    const panel = await openDraftsPanel(page);

    // Both subsection headings should render
    await expect(panel.getByText("Drafts", { exact: true })).toBeVisible({
      timeout: 10_000,
    });
    await expect(panel.getByText("Sent", { exact: true })).toBeVisible({
      timeout: 5_000,
    });

    // At least one row in each subsection
    const draftRows = panel.locator("[data-testid^='home-draft-item-']");
    await expect(draftRows).toHaveCount(3, { timeout: 6_000 });

    // The sent draft content should appear
    await expect(
      panel.getByText("Shipping the draft message improvements", {
        exact: false,
      }),
    ).toBeVisible({ timeout: 5_000 });

    await page.waitForTimeout(200);

    await panel.screenshot({ path: `${SHOTS}/02-sent-subsection.png` });
  });

  test("03 — hover actions visible", async ({ page }) => {
    await installMockBridge(page);
    await patchWorkspacePubkey(page);
    await seedDraftStore(page, ACTIVE_DRAFTS);

    const panel = await openDraftsPanel(page);

    // Wait for the text draft row
    const textDraftRow = panel.locator(
      `[data-testid='home-draft-item-channel:${GENERAL_CHANNEL_ID}']`,
    );
    await expect(textDraftRow).toBeVisible({ timeout: 6_000 });

    // Hover to reveal action buttons
    await textDraftRow.hover();

    // All three action buttons should become visible on hover
    const openDraftBtn = textDraftRow.getByRole("button", {
      name: "Open draft",
      exact: true,
    });
    const sendMessageBtn = textDraftRow.getByRole("button", {
      name: "Send message",
      exact: true,
    });
    const deleteDraftBtn = textDraftRow.getByRole("button", {
      name: "Delete draft",
    });
    await expect(openDraftBtn).toBeVisible({ timeout: 4_000 });
    await expect(sendMessageBtn).toBeVisible({ timeout: 4_000 });
    await expect(deleteDraftBtn).toBeVisible({ timeout: 4_000 });

    await page.waitForTimeout(200);

    await panel.screenshot({ path: `${SHOTS}/03-hover-actions.png` });
  });

  test("04 — empty state", async ({ page }) => {
    await installMockBridge(page);
    // No draft seed → empty state

    const panel = await openDraftsPanel(page);

    // Empty state: FileText icon + "No drafts" text
    await expect(panel.getByText("No drafts")).toBeVisible({ timeout: 5_000 });

    await page.waitForTimeout(200);

    await panel.screenshot({ path: `${SHOTS}/04-empty-state.png` });
  });

  test("05 — thread-draft send confirm dialog", async ({ page }) => {
    // Regression test for IMPORTANT #1: thread-reply draft Send navigates to
    // the correct channel and passes the autoSend key so the thread composer
    // (not the main composer) arms the auto-submit.
    await installMockBridge(page);
    await patchWorkspacePubkey(page);

    // A fixed fake root event ID — in the mock bridge get_event is unhandled
    // so useDraftRootStatus will map it to `error` (not `deleted`), keeping
    // the draft sendable.
    const THREAD_ROOT_ID =
      "aaaa1111bbbb2222cccc3333dddd4444aaaa1111bbbb2222cccc3333dddd4444";
    const THREAD_DRAFT_KEY = `thread:${THREAD_ROOT_ID}`;

    await page.addInitScript(
      ({ storeKey, value }) => {
        window.localStorage.setItem(storeKey, JSON.stringify(value));
      },
      {
        storeKey: DRAFT_STORE_KEY,
        value: {
          [THREAD_DRAFT_KEY]: {
            content: "Thread reply draft content",
            selectionStart: 26,
            selectionEnd: 26,
            channelId: GENERAL_CHANNEL_ID,
            createdAt: CREATED_AT_1,
            updatedAt: CREATED_AT_1,
            pendingImeta: [],
            spoileredAttachmentUrls: [],
            status: "active",
          },
        } satisfies StoredDrafts,
      },
    );

    const panel = await openDraftsPanel(page);

    // The thread draft row should appear.
    const draftRow = panel.locator(
      `[data-testid='home-draft-item-${THREAD_DRAFT_KEY}']`,
    );
    await expect(draftRow).toBeVisible({ timeout: 8_000 });

    // "Thread deleted" label must NOT appear — root status is `error` (optimistic).
    await expect(panel.getByText("Thread deleted")).not.toBeVisible();

    // Hover to reveal the three action buttons.
    await draftRow.hover();
    const sendBtn = draftRow.getByRole("button", {
      name: "Send message",
      exact: true,
    });
    await expect(sendBtn).toBeVisible({ timeout: 4_000 });

    // Click "Send message" — confirm dialog should appear.
    await sendBtn.click();
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible({ timeout: 4_000 });
    await expect(dialog.getByText("Send message")).toBeVisible();
    // Confirm dialog body names the channel.
    await expect(dialog.getByText(/general/i)).toBeVisible();

    await page.waitForTimeout(200);
    await panel.screenshot({
      path: `${SHOTS}/05-thread-draft-send-dialog.png`,
    });

    // Click Send — dialog closes and we navigate to the channel with ?autoSend.
    await dialog.getByRole("button", { name: "Send", exact: true }).click();
    await expect(dialog).not.toBeVisible({ timeout: 4_000 });

    // URL must include all three params set by DraftsPanel.handleConfirmSend:
    //   ?messageId=<rootId>  — scroll-targets the root message in the timeline
    //   ?threadRootId=<rootId> — opens the thread panel for this root
    //   ?autoSend=thread:<rootId> — arms the thread composer's once-only guard
    //
    // Integration boundary: the mock bridge does not support get_event or
    // message send, so ChannelRouteScreen.fetchRouteTargetEvents fails silently,
    // threadHeadMessage stays null, and the MessageThreadPanel (and its composer)
    // never mount. The auto-submit effect and the actual send are NOT assertable
    // in this E2E shard — they are covered by MessageComposerAutoSend.test.mjs
    // (key-match guard) and MessageComposerDraftImagePersist.test.mjs (full
    // composer mount path).
    await expect(page).toHaveURL(new RegExp(`messageId=${THREAD_ROOT_ID}`), {
      timeout: 6_000,
    });
    await expect(page).toHaveURL(new RegExp(`threadRootId=${THREAD_ROOT_ID}`));
    await expect(page).toHaveURL(
      new RegExp(`autoSend=${encodeURIComponent(THREAD_DRAFT_KEY)}`),
    );
  });

  test("06 — active-draft badge on inbox trigger and filter option", async ({
    page,
  }) => {
    // Captures both badge placements for the PR screenshot:
    //   1. The numeric badge on the inbox filter trigger button.
    //   2. The badge next to "Drafts" in the filter dropdown.
    // Two active drafts are seeded so the count is 2.
    await installMockBridge(page);
    await patchWorkspacePubkey(page);
    await seedDraftStore(page, ACTIVE_DRAFTS);

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("home-inbox")).toBeVisible({
      timeout: 10_000,
    });

    // Badge should be visible on the filter trigger with count = 2.
    const triggerBadge = page.getByTestId("inbox-draft-badge");
    await expect(triggerBadge).toBeVisible({ timeout: 6_000 });
    await expect(triggerBadge).toHaveText("2");

    // Open the filter dropdown so the badge-option is visible too.
    await page.getByTestId("inbox-filter-trigger").click();
    const dropdownBadge = page.getByTestId("inbox-draft-badge-option");
    await expect(dropdownBadge).toBeVisible({ timeout: 4_000 });
    await expect(dropdownBadge).toHaveText("2");

    await page.waitForTimeout(200);

    // Capture the full inbox header area including the open dropdown.
    await page.getByTestId("home-inbox").screenshot({
      path: `${SHOTS}/06-draft-badge.png`,
    });

    // Dismiss the dropdown cleanly.
    await page.keyboard.press("Escape");
  });

  test("07 — thread-deleted state (orphaned thread-reply draft)", async ({
    page,
  }) => {
    // A thread-reply draft whose root event is definitively deleted:
    // `useDraftRootStatus` maps it to `deleted`, showing the "Thread deleted"
    // label, greying the row, and disabling Open/Send.
    const DELETED_ROOT_ID =
      "dead0000dead0000dead0000dead0000dead0000dead0000dead0000dead0000";
    const THREAD_DRAFT_KEY = `thread:${DELETED_ROOT_ID}`;

    await installMockBridge(page, { deletedEventIds: [DELETED_ROOT_ID] });
    await patchWorkspacePubkey(page);
    await seedDraftStore(page, {
      [THREAD_DRAFT_KEY]: {
        content: "Planning to follow up on the discussion from last week.",
        selectionStart: 52,
        selectionEnd: 52,
        channelId: GENERAL_CHANNEL_ID,
        createdAt: CREATED_AT_1,
        updatedAt: CREATED_AT_1,
        pendingImeta: [],
        spoileredAttachmentUrls: [],
        status: "active",
      },
    });

    const panel = await openDraftsPanel(page);

    // The orphaned draft row should render.
    const draftRow = panel.locator(
      `[data-testid='home-draft-item-${THREAD_DRAFT_KEY}']`,
    );
    await expect(draftRow).toBeVisible({ timeout: 8_000 });

    // "Thread deleted" badge must appear once the root-status query resolves.
    const orphanLabel = panel.getByTestId(
      `home-draft-orphaned-label-${THREAD_DRAFT_KEY}`,
    );
    await expect(orphanLabel).toBeVisible({ timeout: 8_000 });

    // Hover the row to confirm Open and Send are disabled.
    await draftRow.hover();
    // Both the open and send buttons are labelled "Thread deleted" when orphaned.
    const disabledBtns = draftRow.getByRole("button", {
      name: "Thread deleted",
    });
    await expect(disabledBtns).toHaveCount(2, { timeout: 4_000 });
    // Delete is still enabled.
    await expect(
      draftRow.getByRole("button", { name: "Delete draft" }),
    ).toBeVisible({ timeout: 4_000 });

    await page.waitForTimeout(200);

    await panel.screenshot({
      path: `${SHOTS}/07-thread-deleted-state.png`,
    });
  });
});
