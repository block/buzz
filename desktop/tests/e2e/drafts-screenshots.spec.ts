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

    // Both action buttons should become visible on hover
    const openDraftBtn = textDraftRow.getByRole("button", {
      name: "Open draft",
      exact: true,
    });
    const deleteDraftBtn = textDraftRow.getByRole("button", {
      name: "Delete draft",
    });
    await expect(openDraftBtn).toBeVisible({ timeout: 4_000 });
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
});
