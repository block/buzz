import { expect, test } from "@playwright/test";

import {
  installMockBridge,
  createMockAgentMemoryListing,
} from "../helpers/bridge";

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

// ── (c) Memory gate ordering ────────────────────────────────────────────────

test("team_snapshot_send_memory_gate_blocks_encode_until_confirmed", async ({
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
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  // Open export dialog for the "Engineering" seeded team.
  await page.getByLabel("Engineering team actions").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();

  // Select memory level = "core".
  const coreOption = page.getByRole("radio", {
    name: "Config + core memory",
  });
  if (await coreOption.isVisible()) {
    await coreOption.click();
  }

  // Click "Send in Buzz" to open the send dialog.
  await page.getByTestId("team-snapshot-send-in-buzz").click();
  await expect(page.getByTestId("team-snapshot-send-dialog")).toBeVisible();

  // Select #general.
  await page
    .getByTestId("team-snapshot-send-channel-list")
    .getByText("general")
    .click();

  // Click Next (memory-bearing send shows "Next" instead of "Send").
  await page.getByTestId("team-snapshot-send-confirm").click();

  // The memory gate step must be visible.
  const memGate = page.getByTestId("team-snapshot-send-memgate-confirm");
  await expect(memGate).toBeVisible({ timeout: 3000 });

  // At this point NO encode should have happened — the gate blocks it.
  const logBeforeGate = await readCommandLog(page);
  const encodeBeforeGate = logBeforeGate.filter(
    (e) => e.command === "encode_team_snapshot_for_send",
  );
  expect(encodeBeforeGate).toHaveLength(0);

  // Confirm the memory gate — "Send anyway".
  await memGate.click();

  // Wait for done.
  await expect(page.getByTestId("team-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });

  // Now encode must have been called.
  const logAfterGate = await readCommandLog(page);
  const encodeAfterGate = logAfterGate.filter(
    (e) => e.command === "encode_team_snapshot_for_send",
  );
  expect(encodeAfterGate).toHaveLength(1);

  // The sent message content must use a file-link, not an inline image —
  // a .team.png rendered as ![image](...) produces a blank/invisible message
  // because the PNG body is a 1×1 placeholder.
  const sendEntry = logAfterGate.find(
    (e) => e.command === "send_channel_message",
  );
  expect(sendEntry).toBeTruthy();
  const sendPayload = sendEntry?.payload as { content?: string } | undefined;
  expect(sendPayload?.content).toContain("[e2e-team.team.png](");
  expect(sendPayload?.content).not.toContain("![image](");
});

test("team_snapshot_send_none_memory_skips_gate_and_encodes_directly", async ({
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
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  // Open export dialog for the "Engineering" seeded team.
  await page.getByLabel("Engineering team actions").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();

  // Memory level defaults to "none" — no need to change it.

  // Click "Send in Buzz".
  await page.getByTestId("team-snapshot-send-in-buzz").click();
  await expect(page.getByTestId("team-snapshot-send-dialog")).toBeVisible();

  // Select #general.
  await page
    .getByTestId("team-snapshot-send-channel-list")
    .getByText("general")
    .click();

  // Click Send (no "Next" for none-memory — goes straight to send).
  await page.getByTestId("team-snapshot-send-confirm").click();

  // The memory gate must NOT appear — should go directly to progress/done.
  const memGateBtn = page.getByTestId("team-snapshot-send-memgate-confirm");
  // Give a brief window to ensure memgate doesn't appear.
  await expect(memGateBtn).toHaveCount(0);

  // Wait for done.
  await expect(page.getByTestId("team-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });

  // Encode must have been called directly (no gate).
  const log = await readCommandLog(page);
  const encodeEntries = log.filter(
    (e) => e.command === "encode_team_snapshot_for_send",
  );
  expect(encodeEntries).toHaveLength(1);

  // The sent message content must use a file-link, not an inline image.
  const sendEntry = log.find((e) => e.command === "send_channel_message");
  expect(sendEntry).toBeTruthy();
  const sendPayload = sendEntry?.payload as { content?: string } | undefined;
  expect(sendPayload?.content).toContain("[e2e-team.team.png](");
  expect(sendPayload?.content).not.toContain("![image](");
});
