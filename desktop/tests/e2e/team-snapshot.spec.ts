import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  createMockAgentMemoryListing,
  TEST_IDENTITIES,
} from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

// ── Helpers ───────────────────────────────────────────────────────────────────

type CommandLogEntry = { command: string; payload: unknown };

async function readCommandLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: CommandLogEntry[];
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    );
  });
}

async function gotoAgentsPage(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
}

// Minimal .team.json bytes — the bridge returns a canned preview/result
// regardless of content; these just need to be non-empty so the file-input
// handler invokes handleImportTeamSnapshotFile.
const TEAM_SNAPSHOT_BYTES = new Uint8Array([
  0x7b, 0x22, 0x74, 0x22, 0x3a, 0x31, 0x7d,
]); // {"t":1}

const MOCK_UPLOAD_DESCRIPTOR = {
  url: `https://mock.relay/media/${"a".repeat(64)}.png`,
  sha256: "a".repeat(64),
  size: 1234,
  type: "image/png",
  uploaded: Math.floor(Date.now() / 1000),
  filename: "team.team.png",
};

// Seeded persona ID used across tests.
const ANALYST_PERSONA_ID = "test-analyst";
const ANALYST_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

// ── (a) Confirm-fail + retry ────────────────────────────────────────────────

test("team_snapshot_import_confirm_fail_renders_error_and_retry_succeeds", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: ANALYST_PERSONA_ID,
        displayName: "Analyst",
        systemPrompt: "You are an analyst.",
      },
    ],
    managedAgents: [
      {
        pubkey: ANALYST_PUBKEY,
        name: "Analyst",
        personaId: ANALYST_PERSONA_ID,
      },
    ],
    // First confirm throws, second succeeds.
    teamSnapshotConfirmErrors: ["Relay rejected the import.", null],
  });
  await gotoAgentsPage(page);

  // Trigger import via the hidden file input.
  const fileInput = page.getByTestId("team-snapshot-import-input");
  await fileInput.setInputFiles({
    name: "test.team.json",
    mimeType: "application/json",
    buffer: Buffer.from(TEAM_SNAPSHOT_BYTES),
  });

  // The import dialog must appear with the preview.
  const dialog = page.getByTestId("team-snapshot-import-dialog");
  await expect(dialog).toBeVisible({ timeout: 5000 });

  // Click Import — first confirm fails.
  await dialog.getByTestId("team-snapshot-import-confirm").click();

  // The confirm error banner must be visible in the preview phase.
  const errorBanner = dialog.getByTestId("team-snapshot-import-confirm-error");
  await expect(errorBanner).toBeVisible({ timeout: 5000 });
  await expect(errorBanner).toContainText("Relay rejected the import.");

  // The Import button must still be visible (we're back to preview, not stuck).
  const importBtn = dialog.getByTestId("team-snapshot-import-confirm");
  await expect(importBtn).toBeVisible();

  // Click Import again — second confirm succeeds.
  await importBtn.click();

  // The error banner must disappear (cleared before retry).
  await expect(errorBanner).not.toBeVisible({ timeout: 5000 });

  // The dialog transitions to the result phase — "Team imported" title.
  await expect(dialog.getByText("Team imported")).toBeVisible({
    timeout: 5000,
  });
});

// ── (b) Allowlist payload passthrough ───────────────────────────────────────

test("team_snapshot_import_clear_allowlist_sends_keepAllowlist_false", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: ANALYST_PERSONA_ID,
        displayName: "Analyst",
        systemPrompt: "You are an analyst.",
      },
    ],
    managedAgents: [
      {
        pubkey: ANALYST_PUBKEY,
        name: "Analyst",
        personaId: ANALYST_PERSONA_ID,
      },
    ],
    teamSnapshotPreviewHasSourceAllowlist: true,
  });
  await gotoAgentsPage(page);

  const fileInput = page.getByTestId("team-snapshot-import-input");
  await fileInput.setInputFiles({
    name: "test.team.json",
    mimeType: "application/json",
    buffer: Buffer.from(TEAM_SNAPSHOT_BYTES),
  });

  const dialog = page.getByTestId("team-snapshot-import-dialog");
  await expect(dialog).toBeVisible({ timeout: 5000 });

  // The allowlist section must be visible.
  await expect(
    dialog.getByTestId("team-snapshot-import-allowlist-section"),
  ).toBeVisible();

  // Select Clear (default) and click Import.
  await dialog.getByTestId("team-snapshot-import-allowlist-clear").click();
  await dialog.getByTestId("team-snapshot-import-confirm").click();

  // Wait for result phase.
  await expect(dialog.getByText("Team imported")).toBeVisible({
    timeout: 5000,
  });

  // Verify the command log shows confirm_team_snapshot_import with keepAllowlist: false.
  const log = await readCommandLog(page);
  const confirmEntry = log.find(
    (e) => e.command === "confirm_team_snapshot_import",
  );
  expect(confirmEntry).toBeTruthy();
  const confirmPayload = confirmEntry?.payload as
    | { input?: { keepAllowlist?: boolean } }
    | undefined;
  expect(confirmPayload?.input?.keepAllowlist).toBe(false);
});

test("team_snapshot_import_keep_allowlist_sends_keepAllowlist_true", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: ANALYST_PERSONA_ID,
        displayName: "Analyst",
        systemPrompt: "You are an analyst.",
      },
    ],
    managedAgents: [
      {
        pubkey: ANALYST_PUBKEY,
        name: "Analyst",
        personaId: ANALYST_PERSONA_ID,
      },
    ],
    teamSnapshotPreviewHasSourceAllowlist: true,
  });
  await gotoAgentsPage(page);

  const fileInput = page.getByTestId("team-snapshot-import-input");
  await fileInput.setInputFiles({
    name: "test.team.json",
    mimeType: "application/json",
    buffer: Buffer.from(TEAM_SNAPSHOT_BYTES),
  });

  const dialog = page.getByTestId("team-snapshot-import-dialog");
  await expect(dialog).toBeVisible({ timeout: 5000 });

  // Select Keep.
  await dialog.getByTestId("team-snapshot-import-allowlist-keep").click();
  await dialog.getByTestId("team-snapshot-import-confirm").click();

  // Wait for result phase.
  await expect(dialog.getByText("Team imported")).toBeVisible({
    timeout: 5000,
  });

  // Verify keepAllowlist: true in the command log.
  const log = await readCommandLog(page);
  const confirmEntry = log.find(
    (e) => e.command === "confirm_team_snapshot_import",
  );
  expect(confirmEntry).toBeTruthy();
  const confirmPayload = confirmEntry?.payload as
    | { input?: { keepAllowlist?: boolean } }
    | undefined;
  expect(confirmPayload?.input?.keepAllowlist).toBe(true);
});

// ── (c) Sharing parity ──────────────────────────────────────────────────────

test("team sharing uses the people picker and gates memory before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: ANALYST_PERSONA_ID,
        displayName: "Analyst",
        systemPrompt: "You are an analyst.",
      },
    ],
    managedAgents: [
      {
        pubkey: ANALYST_PUBKEY,
        name: "Analyst",
        personaId: ANALYST_PERSONA_ID,
        status: "running",
      },
    ],
    agentMemory: createMockAgentMemoryListing(),
    searchProfiles: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        displayName: "Charlie",
      },
    ],
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Engineering team actions").click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  const shareDialog = page.getByTestId("team-share-dialog");
  await expect(shareDialog).toBeVisible();
  await expect(
    shareDialog.getByRole("heading", { name: "Share Engineering" }),
  ).toBeVisible();

  const search = shareDialog.getByTestId("team-share-recipient-search");
  await expect(search).toBeEnabled({ timeout: 5_000 });
  await search.fill("charlie");
  await page
    .getByTestId(
      `team-share-recipient-option-${TEST_IDENTITIES.charlie.pubkey}`,
    )
    .click();
  await shareDialog.getByTestId("team-share-recipient-access").click();
  await page.getByRole("menuitemradio", { name: "Team + core memory" }).click();
  await shareDialog.getByTestId("team-share-send").click();

  const memoryConfirmation = page.getByTestId("team-share-memory-confirmation");
  await expect(memoryConfirmation).toBeVisible();
  await expect(memoryConfirmation).toContainText("This team includes");
  await expect(memoryConfirmation).toContainText("plaintext core memory");
  const logBeforeConfirmation = await readCommandLog(page);
  const encodeLevelsBeforeConfirmation = logBeforeConfirmation
    .filter((entry) => entry.command === "encode_team_snapshot_for_send")
    .map(
      (entry) =>
        (entry.payload as { memoryLevel?: string } | undefined)?.memoryLevel,
    );
  expect(encodeLevelsBeforeConfirmation).toEqual([]);

  await memoryConfirmation.getByTestId("team-share-memory-confirm").click();
  await expect(page.getByText("Sent a copy of Engineering")).toBeVisible({
    timeout: 8_000,
  });

  const log = await readCommandLog(page);
  expect(
    log.filter(
      (entry) =>
        entry.command === "encode_team_snapshot_for_send" &&
        (entry.payload as { memoryLevel?: string } | undefined)?.memoryLevel ===
          "core",
    ),
  ).toHaveLength(1);
  const sendEntry = log.find(
    (entry) => entry.command === "send_channel_message",
  );
  expect(sendEntry).toBeTruthy();
  const sendPayload = sendEntry?.payload as { content?: string } | undefined;
  expect(sendPayload?.content).toContain("[Engineering](");
  expect(sendPayload?.content).not.toContain("![image](");
});

test("team sharing keeps link copy and export in the shared surface", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await installMockBridge(page, {
    personas: [
      {
        id: ANALYST_PERSONA_ID,
        displayName: "Analyst",
        systemPrompt: "You are an analyst.",
      },
    ],
    managedAgents: [
      {
        pubkey: ANALYST_PUBKEY,
        name: "Analyst",
        personaId: ANALYST_PERSONA_ID,
      },
    ],
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
    uploadDelayMs: 800,
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Engineering team actions").click();
  const menu = page.getByRole("menu");
  await expect(
    menu.getByRole("menuitem", { name: "Export snapshot" }),
  ).toHaveCount(0);
  await menu.getByRole("menuitem", { name: "Share" }).click();

  const shareDialog = page.getByTestId("team-share-dialog");
  await expect(shareDialog.getByTestId("team-share-link-access")).toHaveText(
    "Team only",
  );
  const exportTeamRow = shareDialog.getByTestId("team-share-export");
  const copyLinkButton = shareDialog.getByTestId("team-share-copy-link");
  const recipientSearch = shareDialog.getByTestId(
    "team-share-recipient-search",
  );
  const linkAccess = shareDialog.getByTestId("team-share-link-access");
  const closeButton = shareDialog.getByRole("button", { name: "Close" });
  await waitForAnimations(page);
  await expect(
    copyLinkButton.getByTestId("team-share-copy-link-stage"),
  ).toHaveCSS("position", "relative");
  const idleCopyLinkButtonBox = await copyLinkButton.boundingBox();
  await copyLinkButton.click();
  await expect(copyLinkButton.locator(".sprout-arc-spinner")).toBeVisible();
  await expect(copyLinkButton).toContainText("Copying…");
  await expect(copyLinkButton).toHaveCSS("opacity", "1");
  await expect(recipientSearch).toBeEnabled();
  await expect(linkAccess).toBeEnabled();
  await expect(closeButton).toBeDisabled();
  await expect(exportTeamRow).toBeDisabled();
  await expect(exportTeamRow).toHaveCSS("opacity", "1");
  await page.keyboard.press("Escape");
  await expect(shareDialog).toBeVisible();
  await expect(copyLinkButton).toContainText("Copied");
  await expect(copyLinkButton).toHaveAttribute("data-copy-status", "copied");
  await expect(closeButton).toBeEnabled();
  await expect(exportTeamRow).toBeEnabled();
  await waitForAnimations(page);
  const copiedButtonBox = await copyLinkButton.boundingBox();
  const copiedStateColors = await copyLinkButton
    .getByTestId("team-share-copy-link-state")
    .evaluate((element) => {
      const icon = element.querySelector("svg");
      const label = element.querySelector("span");
      return {
        icon: icon ? getComputedStyle(icon).color : null,
        label: label ? getComputedStyle(label).color : null,
      };
    });
  expect(copiedStateColors.icon).toBe(copiedStateColors.label);
  expect(copiedButtonBox?.width).toBeLessThan(
    idleCopyLinkButtonBox?.width ?? 0,
  );
  expect(
    Math.abs(
      (copiedButtonBox?.x ?? 0) +
        (copiedButtonBox?.width ?? 0) -
        ((idleCopyLinkButtonBox?.x ?? 0) + (idleCopyLinkButtonBox?.width ?? 0)),
    ),
  ).toBeLessThanOrEqual(1);
  await expect(copyLinkButton).toContainText("Copy link", { timeout: 4_000 });
  await expect(copyLinkButton).toHaveAttribute("data-copy-status", "idle");

  const log = await readCommandLog(page);
  const encodeEntry = log.find(
    (entry) => entry.command === "encode_team_snapshot_for_send",
  );
  expect(encodeEntry?.payload).toEqual(
    expect.objectContaining({ memoryLevel: "none", format: "png" }),
  );

  const copiedTeam = await page.evaluate(() => {
    const commands =
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: { html?: string; text?: string };
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? [];
    return commands.findLast(
      (entry) => entry.command === "copy_text_to_clipboard",
    )?.payload;
  });

  await page.keyboard.press("Escape");
  await expect(shareDialog).toHaveCount(0);
  await page.getByTestId("channel-general").click();
  await page.evaluate(async ({ html, text }) => {
    await navigator.clipboard.write([
      new ClipboardItem({
        "text/html": new Blob([html ?? ""], { type: "text/html" }),
        "text/plain": new Blob([text ?? ""], { type: "text/plain" }),
      }),
    ]);
  }, copiedTeam ?? {});
  await page
    .getByTestId("message-composer")
    .locator("[contenteditable='true']")
    .click();
  await page.keyboard.press("ControlOrMeta+V");

  const composerTeamCard = page.getByTestId("composer-team-snapshot-card");
  await expect(composerTeamCard).toBeVisible();
  await expect(composerTeamCard).toContainText("Engineering");
  await expect(composerTeamCard.locator("img")).toHaveCount(0);
  await page.getByTestId("send-message").click();

  const sentTeamCard = page.getByTestId("agent-snapshot-card").last();
  await expect(sentTeamCard).toBeVisible();
  await expect(sentTeamCard).toContainText("Engineering");
  await expect(sentTeamCard).toContainText("Add team");
  await expect(sentTeamCard.locator("img")).toHaveCount(0);

  await page.getByTestId("open-agents-view").click();
  await page.getByLabel("Engineering team actions").click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  await page.getByTestId("team-share-export").click();
  const exportDialog = page.getByTestId("team-snapshot-export-dialog");
  await expect(exportDialog).toBeVisible();
  await expect(
    exportDialog.getByRole("heading", { name: "Export Engineering" }),
  ).toBeVisible();
  await expect(
    exportDialog.getByTestId("team-snapshot-memory-trigger"),
  ).toHaveText("Team only");
  await expect(
    exportDialog.getByTestId("team-snapshot-format-trigger"),
  ).toHaveText("PNG");
});
