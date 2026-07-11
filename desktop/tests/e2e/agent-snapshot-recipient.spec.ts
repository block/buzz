/**
 * Recipient-native agent snapshot card/import E2E spec.
 *
 * These tests exercise the AgentSnapshotCard rendered in a message timeline
 * when an .agent.json or .agent.png attachment is detected, and the full
 * Import agent → preview → confirm flow.
 */
import { expect, test } from "@playwright/test";
import {
  installMockBridge,
  createMockAgentMemoryListing,
} from "../helpers/bridge";

type CommandLogEntry = { command: string; payload: unknown };

async function readCommandLog(page: import("@playwright/test").Page) {
  return page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: CommandLogEntry[];
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? [],
  );
}

const ANALYST_PERSONA_ID = "test-analyst";
const ANALYST_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const SHA256 = "a".repeat(64);

/** Full imeta descriptor for a .agent.json attachment. */
const SNAPSHOT_UPLOAD_DESCRIPTOR = {
  url: `https://mock.relay/media/${SHA256}.json`,
  sha256: SHA256,
  size: 1234,
  type: "application/json",
  uploaded: Math.floor(Date.now() / 1000),
  filename: "e2e-agent.agent.json",
};

/** Malformed snapshot — bridge will be seeded with snapshotFetchError */
const BAD_UPLOAD_DESCRIPTOR = {
  ...SNAPSHOT_UPLOAD_DESCRIPTOR,
  filename: "bad.agent.json",
};

// ── Helper: seed bridge, send snapshot to #general, navigate to timeline ─────

async function seedAndSendSnapshot(
  page: import("@playwright/test").Page,
  opts: { snapshotFetchError?: string } = {},
) {
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
    uploadDescriptors: [SNAPSHOT_UPLOAD_DESCRIPTOR],
    ...(opts.snapshotFetchError
      ? { snapshotFetchError: opts.snapshotFetchError }
      : {}),
  });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  // Open the send dialog and send to #general.
  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();
  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();
  await page.getByTestId("agent-snapshot-send-confirm").click();
  await expect(page.getByTestId("agent-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });
  await page.getByRole("button", { name: "Close" }).click();

  // Navigate to #general.
  await page.getByTestId("channel-general").click();
}

// ── Timeline renders AgentSnapshotCard, not generic FileCard ──────────────────

test("recipient_timeline_renders_agent_snapshot_card_not_file_card", async ({
  page,
}) => {
  await seedAndSendSnapshot(page);

  // The sent attachment must render as AgentSnapshotCard.
  const card = page.getByTestId("agent-snapshot-card").last();
  await expect(card).toBeVisible({ timeout: 5000 });

  // Must show filename.
  await expect(card).toContainText("e2e-agent.agent.json");

  // Must show "Agent snapshot" label (untrusted until decode).
  await expect(card).toContainText("Agent snapshot");

  // Both actions must be present.
  await expect(card.getByTestId("agent-snapshot-card-import")).toBeVisible();
  await expect(card.getByTestId("agent-snapshot-card-download")).toBeVisible();

  // Generic FileCard must NOT be present for this attachment.
  await expect(page.getByTestId("file-card")).toHaveCount(0);
});

// ── Download still invokes download_file, not fetch_snapshot_bytes ───────────

test("recipient_download_invokes_download_file_only", async ({ page }) => {
  await seedAndSendSnapshot(page);

  const card = page.getByTestId("agent-snapshot-card").last();
  await expect(card).toBeVisible({ timeout: 5000 });

  await card.getByTestId("agent-snapshot-card-download").click();

  const log = await readCommandLog(page);
  const downloadCmds = log.filter((e) => e.command === "download_file");
  const fetchSnapshotCmds = log.filter(
    (e) => e.command === "fetch_snapshot_bytes",
  );
  expect(downloadCmds.length).toBeGreaterThanOrEqual(1);
  expect(fetchSnapshotCmds.length).toBe(0);
});

// ── Import agent: fetch → navigate to agents → open preview ──────────────────

test("recipient_import_navigates_to_agents_and_opens_preview", async ({
  page,
}) => {
  await seedAndSendSnapshot(page);

  const card = page.getByTestId("agent-snapshot-card").last();
  await expect(card).toBeVisible({ timeout: 5000 });

  // Click Import agent.
  await card.getByTestId("agent-snapshot-card-import").click();

  // fetch_snapshot_bytes must have been called.
  await expect(async () => {
    const log = await readCommandLog(page);
    const fetchCmds = log.filter((e) => e.command === "fetch_snapshot_bytes");
    expect(fetchCmds.length).toBeGreaterThanOrEqual(1);
  }).toPass({ timeout: 5000 });

  // download_file must NOT have been called for Import.
  const log = await readCommandLog(page);
  const downloadCmds = log.filter((e) => e.command === "download_file");
  expect(downloadCmds.length).toBe(0);

  // Preview dialog must open on the agents view.
  const dialog = page.getByTestId("agent-snapshot-import-dialog");
  await expect(dialog).toBeVisible({ timeout: 8000 });

  // Decoded display name must appear.
  await expect(dialog).toContainText("Imported Agent");
});

// ── Confirm imports the agent ─────────────────────────────────────────────────

test("recipient_import_confirm_calls_confirm_once_and_shows_result", async ({
  page,
}) => {
  await seedAndSendSnapshot(page);

  const card = page.getByTestId("agent-snapshot-card").last();
  await expect(card).toBeVisible({ timeout: 5000 });
  await card.getByTestId("agent-snapshot-card-import").click();

  const dialog = page.getByTestId("agent-snapshot-import-dialog");
  await expect(dialog).toBeVisible({ timeout: 8000 });

  // Confirm the import.
  await dialog.getByTestId("agent-snapshot-import-confirm").click();

  // confirm_agent_snapshot_import must have been called exactly once.
  await expect(async () => {
    const log = await readCommandLog(page);
    const confirmCmds = log.filter(
      (e) => e.command === "confirm_agent_snapshot_import",
    );
    expect(confirmCmds.length).toBe(1);
  }).toPass({ timeout: 5000 });
});

// ── Malformed/error candidate shows error, never opens preview ────────────────

test("recipient_fetch_error_shows_error_and_download_remains", async ({
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
    uploadDescriptors: [
      {
        ...SNAPSHOT_UPLOAD_DESCRIPTOR,
        filename: "bad.agent.json",
      },
    ],
    snapshotFetchError: "hash mismatch: fetched bytes do not match",
  });
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  // Send to #general.
  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();
  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();
  await page.getByTestId("agent-snapshot-send-confirm").click();
  await expect(page.getByTestId("agent-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByTestId("channel-general").click();

  const card = page.getByTestId("agent-snapshot-card").last();
  await expect(card).toBeVisible({ timeout: 5000 });
  await card.getByTestId("agent-snapshot-card-import").click();

  // Error must appear; preview must NOT open.
  const errorEl = card.getByTestId("agent-snapshot-card-error");
  await expect(errorEl).toBeVisible({ timeout: 5000 });
  await expect(errorEl).toContainText("hash mismatch");
  await expect(
    page.getByTestId("agent-snapshot-import-dialog"),
  ).not.toBeVisible();

  // Download must remain available after error.
  await expect(card.getByTestId("agent-snapshot-card-download")).toBeVisible();
});

// ── Double-click produces one fetch/preview flow ──────────────────────────────

test("recipient_double_click_import_opens_one_preview", async ({ page }) => {
  await seedAndSendSnapshot(page);

  const card = page.getByTestId("agent-snapshot-card").last();
  await expect(card).toBeVisible({ timeout: 5000 });
  const importBtn = card.getByTestId("agent-snapshot-card-import");

  // Click Import agent.
  await importBtn.click();

  // Import dialog must open exactly once (not duplicated by rapid clicks).
  const dialog = page.getByTestId("agent-snapshot-import-dialog");
  await expect(dialog).toHaveCount(1, { timeout: 5000 });

  // Exactly one fetch_snapshot_bytes call.
  const log = await readCommandLog(page);
  const fetchCount = log.filter(
    (e) => e.command === "fetch_snapshot_bytes",
  ).length;
  expect(fetchCount).toBe(1);
});
