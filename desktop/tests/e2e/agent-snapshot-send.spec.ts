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

// Seeded persona ID used across tests.
const ANALYST_PERSONA_ID = "test-analyst";
const ANALYST_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

const MOCK_UPLOAD_DESCRIPTOR = {
  url: `https://mock.relay/media/${"a".repeat(64)}.png`,
  sha256: "a".repeat(64),
  size: 1234,
  type: "image/png",
  uploaded: Math.floor(Date.now() / 1000),
  filename: "analyst.agent.png",
};

// ── Destination picker: channel/DM visibility ─────────────────────────────────

test("snapshot_send_dialog_shows_joined_channels_and_dms", async ({ page }) => {
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  // Pick the "Send in Buzz" option (if shown via format modal) or directly
  // expect the dialog. The export dialog offers save vs send — click Send in Buzz.
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) {
    await sendBtn.click();
  }

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  const list = page.getByTestId("agent-snapshot-send-channel-list");
  await expect(list).toBeVisible();

  // Joined streams (general, random) must appear.
  await expect(list).toContainText("general");
  await expect(list).toContainText("random");

  // DMs (alice-tyler, bob-tyler) must appear.
  await expect(list).toContainText("alice-tyler");
  await expect(list).toContainText("bob-tyler");
});

test("snapshot_send_dialog_excludes_forum_archived_and_moderation_dm", async ({
  page,
}) => {
  // RELAY_SELF_PUBKEY matches alice (the DM peer) so alice-tyler becomes a moderation DM.
  const ALICE_PUBKEY =
    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
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
    relaySelf: ALICE_PUBKEY,
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  const list = page.getByTestId("agent-snapshot-send-channel-list");
  await expect(list).toBeVisible();

  // The DM whose only other participant is ALICE_PUBKEY (= relaySelf) must
  // NOT appear — it is a moderation DM.
  await expect(list.getByText("alice-tyler")).toHaveCount(0);

  // Forums must not appear (watercooler and announcements are seeded forums).
  await expect(list).not.toContainText("watercooler");
  await expect(list).not.toContainText("announcements");

  // Non-member channels must not appear (design and sales exclude the mock user).
  await expect(list).not.toContainText("design");
  await expect(list).not.toContainText("sales");
});

// ── Moderation-DM fail-closed race: DMs withheld during relay-self loading ────

test("snapshot_send_moderation_dm_not_selectable_during_relay_self_loading", async ({
  page,
}) => {
  // ALICE_PUBKEY is relaySelf but the response is delayed — alice-tyler must
  // NOT appear in the picker while get_relay_self is in-flight.
  const ALICE_PUBKEY =
    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
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
    relaySelf: ALICE_PUBKEY,
    // Delay get_relay_self so we can observe the fail-closed window.
    relaySelfDelayMs: 2000,
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  const list = page.getByTestId("agent-snapshot-send-channel-list");
  await expect(list).toBeVisible();

  // While relay-self is loading, ALL DMs must be withheld (fail-closed).
  // alice-tyler, bob-tyler, and the generic "DM" channel must not appear.
  await expect(list.getByText("alice-tyler")).toHaveCount(0);
  await expect(list.getByText("bob-tyler")).toHaveCount(0);
  await expect(list.getByText("charlie")).toHaveCount(0);

  // Attempting to confirm must not be possible (no DM is selectable).
  // Streams (general, random) are still available — confirm that.
  await expect(list).toContainText("general");

  // Select general and confirm — must proceed to done without any DM encode.
  await list.getByText("general").click();
  await page.getByTestId("agent-snapshot-send-confirm").click();
  await expect(page.getByTestId("agent-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });

  // No DM channel was sent to.
  const log = await readCommandLog(page);
  const sendEntry = log.find((e) => e.command === "send_channel_message");
  const sendPayload = sendEntry?.payload as { channelId?: string } | undefined;
  // general's id is the well-known seed value.
  expect(sendPayload?.channelId).toBe("9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50");
});

// ── Config-only send flow: encode → upload → send, correct destination ────────

test("snapshot_send_config_only_calls_encode_upload_send_in_order", async ({
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select #general.
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();

  // Confirm send.
  await page.getByTestId("agent-snapshot-send-confirm").click();

  // Dialog transitions: progress → done.
  await expect(page.getByTestId("agent-snapshot-send-progress")).toBeVisible();
  await expect(page.getByTestId("agent-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });

  // Verify command order: encode → upload → send_channel_message.
  const log = await readCommandLog(page);
  const relevantCommands = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);

  expect(relevantCommands).toEqual([
    "encode_agent_snapshot_for_send",
    "upload_media_bytes",
    "send_channel_message",
  ]);

  // Confirm send_channel_message targeted #general (its id from the seed).
  const sendEntry = log.find((e) => e.command === "send_channel_message");
  expect(sendEntry).toBeTruthy();
  const sendPayload = sendEntry?.payload as
    | { channelId?: string; mediaTags?: string[][] }
    | undefined;
  // The general channel id is fixed in the e2eBridge seed.
  expect(sendPayload?.channelId).toBe("9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50");

  // The imeta tag must carry exact URL / MIME / hash / size / filename from
  // the mock upload descriptor — proves the descriptor is threaded through
  // buildImetaTags without field drops or substitutions.
  const imeta = sendPayload?.mediaTags?.[0];
  expect(imeta).toBeDefined();
  const sha = "a".repeat(64);
  const expectedUrl = `https://mock.relay/media/${sha}.png`;
  expect(imeta).toContain(`url ${expectedUrl}`);
  expect(imeta).toContain("m image/png");
  expect(imeta).toContain(`x ${sha}`);
  expect(imeta).toContain("size 1234");
  // The filename in the imeta comes from the encode payload's fileName — the
  // controller sets descriptorWithFilename.filename = fileName (the file produced
  // by encode_agent_snapshot_for_send), which the bridge hardcodes as
  // "e2e-agent.agent.png".
  expect(imeta).toContain("filename e2e-agent.agent.png");

  // Send-in-Buzz always encodes PNG so the attachment itself provides the
  // avatar thumbnail.
  const encodeEntry = log.find(
    (e) => e.command === "encode_agent_snapshot_for_send",
  );
  expect(encodeEntry).toBeTruthy();
  expect(
    (encodeEntry?.payload as { format?: string } | undefined)?.format,
  ).toBe("png");

  // Close the dialog and navigate to #general to verify the AgentSnapshotCard renders.
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByTestId("channel-general").click();

  // The sent attachment must appear as an AgentSnapshotCard (not a generic
  // FileCard) with the exact filename that the encode step produced.
  const snapshotCard = page.getByTestId("agent-snapshot-card").last();
  await expect(snapshotCard).toBeVisible({ timeout: 5000 });
  await expect(snapshotCard).toContainText("e2e-agent.agent.png");
});

// ── Memory-bearing flow: gate stops before encode/upload/send ─────────────────

test("snapshot_send_memory_gate_stops_before_encode_on_cancel", async ({
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();

  // Select memory level = "core" using the accessible label on the radio input.
  // The radio inputs are wrapped in <label> elements whose text contains the
  // option name; getByRole with a name finds the input via its accessible name.
  const coreOption = page.getByRole("radio", {
    name: "Config + core memory",
  });
  if (await coreOption.isVisible()) {
    await coreOption.click();
  }
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select a destination.
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();
  await page.getByTestId("agent-snapshot-send-confirm").click();

  // The memory gate MUST be visible when core memory is selected — fail if not.
  const memGate = page.getByTestId("agent-snapshot-send-memory-gate");
  await expect(memGate).toBeVisible({ timeout: 3000 });

  // Cancel — must NOT trigger encode/upload/send.
  await page.getByRole("button", { name: "Cancel" }).click();

  // Verify NO encode/upload/send was called before or after cancel.
  const log = await readCommandLog(page);
  const dangerCmds = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);
  expect(dangerCmds).toEqual([]);
});

// ── Memory-bearing flow: gate names resolved destination ──────────────────────

test("snapshot_send_memory_gate_names_the_destination", async ({ page }) => {
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();

  // Select memory level = "core".
  const coreOption = page.getByRole("radio", {
    name: "Config + core memory",
  });
  if (await coreOption.isVisible()) {
    await coreOption.click();
  }
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select #general.
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();
  await page.getByTestId("agent-snapshot-send-confirm").click();

  // The memory gate must be visible and must name both the destination and
  // the media-link risk.
  const memGate = page.getByTestId("agent-snapshot-send-memory-gate");
  await expect(memGate).toBeVisible({ timeout: 3000 });
  await expect(memGate).toContainText("#general");
  await expect(memGate).toContainText("media link");
});

// ── Generic DM label: resolved label used consistently ────────────────────────

test("snapshot_send_generic_dm_shows_resolved_participant_label", async ({
  page,
}) => {
  // The generic "DM" channel with charlie must show "charlie" as the label in
  // picker, search filter, memory-gate warning, and done copy — not "DM".
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();

  // Select memory level = "core" so the memory gate shows the destination label.
  const coreOption = page.getByRole("radio", {
    name: "Config + core memory",
  });
  if (await coreOption.isVisible()) {
    await coreOption.click();
  }
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  const list = page.getByTestId("agent-snapshot-send-channel-list");

  // The generic DM must appear in the picker with the resolved label "charlie",
  // NOT as bare "DM".
  await expect(list).toContainText("charlie");

  // Search by resolved label must find it.
  await page.getByTestId("agent-snapshot-send-search").fill("charlie");
  await expect(list).toContainText("charlie");

  // Select it.
  await list.getByText("charlie").first().click();

  // Clear search and confirm.
  await page.getByTestId("agent-snapshot-send-search").fill("");
  await page.getByTestId("agent-snapshot-send-confirm").click();

  // Memory gate must name the destination using the resolved label, not "DM".
  const memGate = page.getByTestId("agent-snapshot-send-memory-gate");
  await expect(memGate).toBeVisible({ timeout: 3000 });
  await expect(memGate).toContainText("charlie");
  await expect(memGate).not.toContainText('"DM"');

  // Confirm send — done copy must also use the resolved label.
  await page.getByTestId("agent-snapshot-send-memgate-confirm").click();

  const done = page.getByTestId("agent-snapshot-send-done");
  await expect(done).toBeVisible({ timeout: 8000 });
  const doneText = (await done.textContent()) ?? "";
  // Done copy must mention "charlie" — proves the resolved label is used, not
  // the raw generic channel name "DM".
  expect(doneText).toMatch(/charlie/);
  // Must not say "sent to DM" using just the raw channel name.
  expect(doneText).not.toMatch(/sent to DM\b/i);
});

// ── Timeout gate: timed-out user cannot encode/upload/send ───────────────────

test("snapshot_send_timeout_gate_blocks_encode_before_upload", async ({
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

  // Activate the timeout store after the app has loaded via the test-only
  // bridge global — mirrors a post-send rejection that flips active=true.
  await page.evaluate(() => {
    const expiryMs = Date.now() + 60_000; // 60 s from now
    (
      window as Window & {
        __BUZZ_E2E_ACTIVATE_TIMEOUT__?: (ms: number) => void;
      }
    ).__BUZZ_E2E_ACTIVATE_TIMEOUT__?.(expiryMs);
  });

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

  // The send must fail with an error — the timeout guard fires before encode.
  const errorEl = page.getByTestId("agent-snapshot-send-error");
  await expect(errorEl).toBeVisible({ timeout: 5000 });

  // Verify zero encode/upload/send were called.
  const log = await readCommandLog(page);
  const dangerCmds = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);
  expect(dangerCmds).toEqual([]);
});

// ── Progress phases: Preparing → Uploading → Sending ─────────────────────────

test("snapshot_send_progress_shows_preparing_uploading_sending_phases", async ({
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
    // Delay encode so "Preparing snapshot…" is observable before "Uploading".
    encodeDelayMs: 400,
    // Delay upload so "Uploading snapshot…" is observable before "Sending".
    uploadDelayMs: 400,
    // Delay send so "Sending message…" is observable before done.
    sendMessageDelayMs: 400,
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

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

  // The progress step must appear and show the Preparing phase BEFORE the
  // encode delay completes — this verifies the controller sets "preparing"
  // before calling encode, not after.
  const progress = page.getByTestId("agent-snapshot-send-progress");
  await expect(progress).toBeVisible();
  await expect(progress).toHaveText("Preparing snapshot…");

  // After encoding, the progress label transitions to Uploading.
  await expect(progress).toHaveText("Uploading snapshot…", { timeout: 5000 });

  // After upload, the progress label transitions to Sending.
  await expect(progress).toHaveText("Sending message…", { timeout: 5000 });

  // Wait for done.
  await expect(page.getByTestId("agent-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });
});

// ── Done copy: no claim of direct import ─────────────────────────────────────

test("snapshot_send_done_does_not_claim_direct_import", async ({ page }) => {
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

  const done = page.getByTestId("agent-snapshot-send-done");
  await expect(done).toBeVisible({ timeout: 8000 });

  // Must NOT claim recipients can directly click to import.
  const doneText = (await done.textContent()) ?? "";
  expect(doneText).not.toMatch(/click.*import/i);
  expect(doneText).not.toMatch(/directly from the message/i);

  // Must name the destination.
  expect(doneText).toMatch(/#general|general/);
});

// ── Double-send guard: confirming twice cannot duplicate ──────────────────────

test("snapshot_send_double_confirm_cannot_duplicate_send", async ({ page }) => {
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
    // Slow encode + upload so both clicks happen before the first operation
    // resolves — proves the in-flight guard, not just button disappearance.
    encodeDelayMs: 400,
    uploadDelayMs: 400,
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();

  const confirmBtn = page.getByTestId("agent-snapshot-send-confirm");

  // First click starts the encode (in-flight due to encodeDelayMs: 400).
  await confirmBtn.click();

  // The progress indicator must appear — we're in the encode phase.
  // The confirm button is gone (dialog switched to progress step), so no
  // second click is possible from the UI — the guard is the step transition.
  const progress = page.getByTestId("agent-snapshot-send-progress");
  await expect(progress).toBeVisible();
  // Confirm the button is no longer present — this is the UI guard.
  await expect(confirmBtn).toHaveCount(0);

  await expect(page.getByTestId("agent-snapshot-send-done")).toBeVisible({
    timeout: 8000,
  });

  // send_channel_message must have been invoked exactly once.
  const log = await readCommandLog(page);
  const sendCount = log.filter(
    (e) => e.command === "send_channel_message",
  ).length;
  expect(sendCount).toBe(1);

  // encode must also have been called exactly once.
  const encodeCount = log.filter(
    (e) => e.command === "encode_agent_snapshot_for_send",
  ).length;
  expect(encodeCount).toBe(1);
});

// ── Group DM: resolved label used in picker/search/memgate/done ──────────────

test("snapshot_send_group_dm_shows_resolved_participant_labels", async ({
  page,
}) => {
  // The "Group DM (3)" channel with alice+bob+tyler must show "alice, bob" as
  // the label in picker, search, memory-gate warning, and done — not "Group DM (3)".
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();

  // Select memory level = "core" so the memory gate shows the destination label.
  const coreOption = page.getByRole("radio", {
    name: "Config + core memory",
  });
  if (await coreOption.isVisible()) {
    await coreOption.click();
  }
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  const list = page.getByTestId("agent-snapshot-send-channel-list");

  // The Group DM must appear in the picker with the resolved label "bob, charlie",
  // NOT as bare "Group DM (3)".
  await expect(list).toContainText("bob, charlie");

  // Search by resolved label must find it.
  await page.getByTestId("agent-snapshot-send-search").fill("bob");
  await expect(list).toContainText("bob, charlie");

  // Select it.
  await list.getByText("bob, charlie").first().click();

  // Clear search and confirm.
  await page.getByTestId("agent-snapshot-send-search").fill("");
  await page.getByTestId("agent-snapshot-send-confirm").click();

  // Memory gate must name the destination using the resolved label.
  const memGate = page.getByTestId("agent-snapshot-send-memory-gate");
  await expect(memGate).toBeVisible({ timeout: 3000 });
  await expect(memGate).toContainText("bob, charlie");
  await expect(memGate).not.toContainText("Group DM");

  // Confirm send — done copy must also use the resolved label.
  await page.getByTestId("agent-snapshot-send-memgate-confirm").click();

  const done = page.getByTestId("agent-snapshot-send-done");
  await expect(done).toBeVisible({ timeout: 8000 });
  const doneText = (await done.textContent()) ?? "";
  expect(doneText).toMatch(/bob/);
  expect(doneText).toMatch(/charlie/);
  expect(doneText).not.toMatch(/Group DM/);
});

// ── Live destination invalidation: post-selection mutation blocks encode ──────

test("snapshot_send_archived_destination_blocks_encode_after_selection", async ({
  page,
}) => {
  // Seeded with an encodeDelayMs so the confirm button is still present when
  // the test inspects it (archive mutation happens before confirm click).
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select #random.
  const list = page.getByTestId("agent-snapshot-send-channel-list");
  await list.getByText("random").click();

  // Archive #random after selection via the test-only command bridge.
  await page.evaluate(() => {
    return (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          cmd: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("archive_channel", {
      channelId: "9dae0116-799b-5071-a0a8-fdd30a91a35d",
    });
  });

  // Invalidate the channels cache so the mutation is visible to subscribers.
  await page.evaluate(() => {
    return (
      window as Window & {
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  });

  // Wait for the confirm button to become disabled (selection cleared because
  // #random left sendableChannels) or for the error step to appear.
  // Either outcome proves zero encode/upload/send.
  const confirmBtn = page.getByTestId("agent-snapshot-send-confirm");

  // The confirm button must be disabled (no selection) within 3 s.
  await expect(confirmBtn).toBeDisabled({ timeout: 3000 });

  // Attempting to confirm does nothing — button is disabled.
  // Verify zero encode/upload/send.
  const log = await readCommandLog(page);
  const dangerCmds = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);
  expect(dangerCmds).toEqual([]);
});

// ── Live invalidation: membership loss blocks encode ──────────────────────────

test("snapshot_send_membership_loss_blocks_encode_after_selection", async ({
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select #engineering (the mock identity is a member).
  const list = page.getByTestId("agent-snapshot-send-channel-list");
  await list.getByText("engineering").click();

  // Remove the mock identity from #engineering's membership via direct mutation.
  const MOCK_PUBKEY = "deadbeef".repeat(8);
  await page.evaluate((pubkey) => {
    (
      window as Window & {
        __BUZZ_E2E_MUTATE_CHANNEL__?: (opts: {
          channelId: string;
          removeMemberPubkey?: string;
        }) => void;
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_MUTATE_CHANNEL__?.({
      channelId: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
      removeMemberPubkey: pubkey,
    });
    return (
      window as Window & {
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  }, MOCK_PUBKEY);

  // Confirm button must become disabled (selection cleared by the effect).
  const confirmBtn = page.getByTestId("agent-snapshot-send-confirm");
  await expect(confirmBtn).toBeDisabled({ timeout: 3000 });

  // Zero encode/upload/send — handleSend was never reached.
  const log = await readCommandLog(page);
  const dangerCmds = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);
  expect(dangerCmds).toEqual([]);
});

// ── Live invalidation: stream→forum conversion blocks encode ──────────────────

test("snapshot_send_forum_conversion_blocks_encode_after_selection", async ({
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

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select #agents (stream, mock identity is a member).
  const list = page.getByTestId("agent-snapshot-send-channel-list");
  await list.getByText("agents").click();

  // Convert #agents from stream to forum after selection.
  await page.evaluate(() => {
    (
      window as Window & {
        __BUZZ_E2E_MUTATE_CHANNEL__?: (opts: {
          channelId: string;
          channelType?: "stream" | "forum" | "dm";
        }) => void;
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_MUTATE_CHANNEL__?.({
      channelId: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
      channelType: "forum",
    });
    return (
      window as Window & {
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  });

  // Confirm button must become disabled (selection cleared — forums excluded).
  const confirmBtn = page.getByTestId("agent-snapshot-send-confirm");
  await expect(confirmBtn).toBeDisabled({ timeout: 3000 });

  // Zero encode/upload/send — handleSend was never reached.
  const log = await readCommandLog(page);
  const dangerCmds = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);
  expect(dangerCmds).toEqual([]);
});

// ── Moderation-DM action boundary: confirm is disabled during relay-self load ─

test("snapshot_send_moderation_target_confirm_disabled_during_relay_self_load", async ({
  page,
}) => {
  // ALICE_PUBKEY will be relaySelf (making alice-tyler a moderation DM), but
  // the response is delayed. During the loading window:
  // - All DMs are withheld from the picker (fail-closed)
  // - The confirm button is disabled (no valid selection possible for a DM)
  // - Zero encode/upload/send can be triggered for that DM target
  const ALICE_PUBKEY =
    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
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
    relaySelf: ALICE_PUBKEY,
    // Long delay keeps the loading window open for the full assertion window.
    relaySelfDelayMs: 5000,
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  const list = page.getByTestId("agent-snapshot-send-channel-list");

  // alice-tyler (the moderation-DM target) must NOT appear — fail-closed.
  await expect(list.getByText("alice-tyler")).toHaveCount(0);

  // The confirm button must be disabled (no selection is possible for the
  // moderation-DM target while classification is unresolved).
  const confirmBtn = page.getByTestId("agent-snapshot-send-confirm");
  await expect(confirmBtn).toBeDisabled();

  // Zero encode/upload/send for the moderation-DM target.
  const log = await readCommandLog(page);
  const dangerCmds = log
    .filter((e) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(e.command),
    )
    .map((e) => e.command);
  expect(dangerCmds).toEqual([]);
});

// ── Adversarial: eligibility checkpoint 2 — mutate during encode, no upload ──

test("snapshot_send_dest_invalidated_during_encode_blocks_upload", async ({
  page,
}) => {
  // This test exercises the second eligibility checkpoint in beginSend:
  // after encode completes but before upload starts.
  //
  // Setup: use encodeDelayMs: 600 so there is a window to mutate the
  // destination while encode is in-flight.  The test:
  //   1. Confirms send (pre-flight passes, guard acquired, encode starts).
  //   2. Mutates #general to a forum type while encode is in-flight.
  //   3. When encode completes, checkpoint 2 fires → returns error string.
  //   4. Asserts: encode ran once, upload and send never ran.
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
    // Slow encode gives us time to mutate the destination mid-flight.
    encodeDelayMs: 600,
    uploadDescriptors: [MOCK_UPLOAD_DESCRIPTOR],
  });
  await gotoAgentsPage(page);

  await page.getByLabel("Open actions for Analyst").click();
  await page.getByRole("menuitem", { name: "Export snapshot" }).click();
  const sendBtn = page.getByRole("button", { name: "Send in Buzz" });
  if (await sendBtn.isVisible()) await sendBtn.click();

  await expect(page.getByTestId("agent-snapshot-send-dialog")).toBeVisible();

  // Select #general.
  await page
    .getByTestId("agent-snapshot-send-channel-list")
    .getByText("general")
    .click();

  // Click confirm — pre-flight passes, progress step visible, encode in-flight.
  await page.getByTestId("agent-snapshot-send-confirm").click();
  const progress = page.getByTestId("agent-snapshot-send-progress");
  await expect(progress).toBeVisible();
  await expect(progress).toHaveText("Preparing snapshot…");

  // While encode is in-flight, convert #general to a forum.
  // This makes it ineligible: isSendableDestination returns false for forums.
  await page.evaluate(() => {
    (
      window as Window & {
        __BUZZ_E2E_MUTATE_CHANNEL__?: (opts: {
          channelId: string;
          channelType?: "stream" | "forum" | "dm";
        }) => void;
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_MUTATE_CHANNEL__?.({
      channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
      channelType: "forum",
    });
    return (
      window as Window & {
        __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      }
    ).__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  });

  // Encode completes, checkpoint 2 fires and detects the invalid destination.
  // The dialog must show the error step (never transitions to "Uploading").
  const errorEl = page.getByTestId("agent-snapshot-send-error");
  await expect(errorEl).toBeVisible({ timeout: 5000 });

  // Verify: encode ran once, upload and send never ran.
  const log = await readCommandLog(page);
  const encodeCount = log.filter(
    (e) => e.command === "encode_agent_snapshot_for_send",
  ).length;
  const uploadCount = log.filter(
    (e) => e.command === "upload_media_bytes",
  ).length;
  const sendCount = log.filter(
    (e) => e.command === "send_channel_message",
  ).length;

  expect(encodeCount).toBe(1); // encode ran — checkpoint 1 passed
  expect(uploadCount).toBe(0); // upload blocked by checkpoint 2
  expect(sendCount).toBe(0); // send never reached
});
