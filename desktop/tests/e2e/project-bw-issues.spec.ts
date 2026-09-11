import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// P4D: BW-enrolled issue creation, text/triage writes and the fixed
// action-needed views. The mocked `get_project_bw` / `submit_project_bw_record`
// commands here perform no Core validation of their own — that authority is
// exclusively `buzz-core`/`buzz-sdk`'s job, proven against the pinned NIP-BW
// fixture corpus (crates/buzz-sdk/src/bw.rs,
// desktop/src-tauri/src/commands/project_bw_write.rs). These specs prove the
// desktop UI: signs the root plus a deliberate enroll transition and never a
// bare root; blocks a doomed submit (missing criteria, a double-click) before
// any call; and surfaces a Core refusal or a technical conflict visibly
// instead of silently retrying, overwriting or pretending success.

const POLICY_ID = "9".repeat(64);
const GENESIS_ID = "8".repeat(64);
const ROOT_ENROLLED = "1".repeat(64);
const ROOT_UNENROLLED = "2".repeat(64);
const ROOT_CONFLICT = "3".repeat(64);
const UPDATE_FORK_A = "4".repeat(64);
const UPDATE_FORK_B = "5".repeat(64);
const REPORTER = "c".repeat(64);

function rootEvent(
  id: string,
  subject: string,
  content = "Fixture description",
) {
  return {
    id,
    pubkey: REPORTER,
    created_at: 1_800_000_000,
    kind: 1621,
    tags: [
      ["a", `30617:${"a".repeat(64)}:fixture-bw`],
      ["subject", subject],
    ],
    content,
    sig: "0".repeat(128),
  };
}

function issueUpdateEvent(id: string, issue: string, title: string) {
  return {
    id,
    pubkey: REPORTER,
    created_at: 1_800_000_100,
    kind: 46100,
    tags: [
      ["record", "issue-update"],
      ["a", `30617:${"a".repeat(64)}:fixture-bw`],
      ["policy", POLICY_ID],
      ["issue", issue],
    ],
    content: JSON.stringify({ patch: { title } }),
    sig: "0".repeat(128),
  };
}

const ACTIVATION = { policy: POLICY_ID, genesis: GENESIS_ID };

async function openIssuesPanel(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-issues-trigger").click();
  const panel = page.getByTestId("channel-issues-auxiliary-pane");
  await expect(panel).toBeVisible();
  return panel;
}

test("a root without any enroll attempt is shown as not yet a BW issue, never a false Triage entry", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT_UNENROLLED]: rootEvent(ROOT_UNENROLLED, "Unenrolled root"),
      },
      // No issue-state record at all: Core's own projection never places an
      // un-enrolled root in `issues`, so this stays empty here too.
      projection: { issues: {} },
    },
  });
  const panel = await openIssuesPanel(page);

  const row = panel.locator(`[data-project-event-id="${ROOT_UNENROLLED}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();

  await expect(panel.getByTestId("bw-not-enrolled")).toBeVisible();
  await expect(panel.getByTestId("bw-not-enrolled")).toContainText(
    "Not yet a BW issue",
  );
  await expect(panel.getByTestId("bw-enroll-issue")).toBeVisible();
  // No triage/text-edit affordances for something that isn't actually
  // enrolled — those require `issue.bw.state`, which stays null here.
  await expect(panel.getByTestId("bw-triage-actions")).toHaveCount(0);
  await expect(panel.getByTestId("bw-text-editor")).toHaveCount(0);
});

test("a concurrent text-update fork surfaces a visible conflict, never a silent overwrite", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT_CONFLICT]: rootEvent(ROOT_CONFLICT, "Forked issue"),
        [UPDATE_FORK_A]: issueUpdateEvent(
          UPDATE_FORK_A,
          ROOT_CONFLICT,
          "Title A",
        ),
        [UPDATE_FORK_B]: issueUpdateEvent(
          UPDATE_FORK_B,
          ROOT_CONFLICT,
          "Title B",
        ),
      },
      projection: {
        issues: { [ROOT_CONFLICT]: "triage" },
        conflicts: [UPDATE_FORK_A, UPDATE_FORK_B],
      },
    },
  });
  const panel = await openIssuesPanel(page);

  const row = panel.locator(`[data-project-event-id="${ROOT_CONFLICT}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();

  await expect(panel.getByTestId("bw-field-conflict")).toBeVisible();
  await expect(panel.getByTestId("bw-field-conflict")).toContainText(
    "Concurrent text edits are in conflict",
  );
});

test("a triage action refused by Core surfaces the refusal instead of a silent state change", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT_ENROLLED]: rootEvent(ROOT_ENROLLED, "Wrong-role attempt"),
      },
      projection: { issues: { [ROOT_ENROLLED]: "triage" } },
    },
    // The real desktop command would refuse this through Core
    // (`bw:reject:role:unauthorized`, proven in
    // desktop/src-tauri/src/commands/project_bw_write.rs against the pinned
    // fixture corpus); this scripts the same refusal so the UI's handling of
    // it — visible, not silent — is what this spec actually exercises.
    bwSubmitErrors: ["bw:reject:role:unauthorized"],
  });
  const panel = await openIssuesPanel(page);

  const row = panel.locator(`[data-project-event-id="${ROOT_ENROLLED}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();

  await panel.getByTestId("bw-triage-accept").click();
  await expect(page.getByText("bw:reject:role:unauthorized")).toBeVisible();
  // The refused action must not be reflected as a status change.
  await expect(panel.getByTestId("project-issue-status")).toContainText(
    "Triage",
  );
});

test("retrying enrollment against a refusal (e.g. a pre-cutover root) shows the reason, never a false success", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT_UNENROLLED]: rootEvent(ROOT_UNENROLLED, "Pre-cutover root"),
      },
      projection: { issues: {} },
    },
    // Mirrors the real `bw:reject:policy:cutover` Core returns for a root
    // created before its repository's policy cutover (see
    // crates/buzz-core/src/bw/semantics.rs and the pinned
    // `cutover-old-root` fixture case, replayed directly against this
    // command's own code path in project_bw_write.rs's tests).
    bwSubmitErrors: ["bw:reject:policy:cutover"],
  });
  const panel = await openIssuesPanel(page);

  const row = panel.locator(`[data-project-event-id="${ROOT_UNENROLLED}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();
  await panel.getByTestId("bw-enroll-issue").click();

  await expect(page.getByText("bw:reject:policy:cutover")).toBeVisible();
  // Still not enrolled — the refusal must not be swallowed into a fake accept.
  await expect(panel.getByTestId("bw-not-enrolled")).toBeVisible();
});

test("creating a BW issue requires at least one acceptance criterion and submits nothing until then", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: { activation: ACTIVATION },
  });
  const panel = await openIssuesPanel(page);

  await panel.getByRole("button", { name: "Create issue" }).click();
  const dialog = page.getByRole("dialog", { name: "Create an issue" });
  await expect(dialog).toBeVisible();

  await dialog.getByTestId("create-issue-title").fill("Needs criteria");
  const submit = dialog.getByTestId("create-issue-submit");
  // No template selected and no criteria typed: submit stays disabled.
  await expect(submit).toBeDisabled();

  await dialog.getByTestId("create-issue-template").selectOption("bug");
  // The bug template pre-fills a default criterion; clear it back out.
  await dialog.getByTestId("create-issue-acceptance-criteria").fill("");
  await expect(submit).toBeDisabled();

  const commandsBefore = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_PAYLOADS__?.length ?? 0,
  );
  // Force the form's submit event directly, bypassing the disabled submit
  // button entirely — the dialog's own guard
  // (`criteriaRequired && !criteriaFilled` in
  // CreateProjectWorkItemDialog.tsx), not just a disabled attribute, must
  // still refuse this the same way a direct/scripted call would be refused.
  await page.evaluate(() => {
    document
      .querySelector<HTMLFormElement>("form#create-issue-form")
      ?.requestSubmit();
  });
  await expect(dialog).toBeVisible();
  const commandsAfter = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_PAYLOADS__?.length ?? 0,
  );
  expect(commandsAfter).toBe(commandsBefore);
  const signedRoots = await page.evaluate(() =>
    (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).filter((e) => e.kind === 1621),
  );
  expect(signedRoots).toHaveLength(0);
});

test("creating a BW issue signs a 1621 root plus a deliberate enroll transition; a rapid double-click still creates only one", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: { activation: ACTIVATION },
  });
  const panel = await openIssuesPanel(page);

  await panel.getByRole("button", { name: "Create issue" }).click();
  const dialog = page.getByRole("dialog", { name: "Create an issue" });
  await expect(dialog).toBeVisible();

  await dialog.getByTestId("create-issue-title").fill("BW issue via template");
  await dialog.getByTestId("create-issue-template").selectOption("bug");
  await expect(
    dialog.getByTestId("create-issue-acceptance-criteria"),
  ).not.toHaveValue("");
  const submit = dialog.getByTestId("create-issue-submit");
  await expect(submit).toBeEnabled();

  // Dispatch two clicks in the same synchronous turn — the race a real
  // double-click can produce — rather than two separately-awaited
  // Playwright clicks, which always leave a task-boundary gap.
  await submit.evaluate((button: HTMLButtonElement) => {
    button.click();
    button.click();
  });
  await expect(dialog).toHaveCount(0, { timeout: 10_000 });

  const signedRoots = await page.evaluate(() =>
    (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).filter((e) => e.kind === 1621),
  );
  expect(signedRoots).toHaveLength(1);
  expect(signedRoots[0]?.tags).toContainEqual(
    expect.arrayContaining(["subject", "BW issue via template"]),
  );
  // No `p` recipient tag: a BW kind:1621 root only ever carries `a`/`subject`.
  expect(signedRoots[0]?.tags.some((tag) => tag[0] === "p")).toBe(false);

  const bwCalls = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "submit_project_bw_record",
    ),
  );
  const enrollCalls = bwCalls.filter(
    (entry) =>
      (entry.payload as { input?: { record?: string } } | null)?.input
        ?.record === "issue-state",
  );
  expect(enrollCalls).toHaveLength(1);
  expect(
    (
      enrollCalls[0]?.payload as {
        input?: { content?: { state?: string } };
      }
    )?.input?.content?.state,
  ).toBe("triage");
  const updateCalls = bwCalls.filter(
    (entry) =>
      (entry.payload as { input?: { record?: string } } | null)?.input
        ?.record === "issue-update",
  );
  expect(updateCalls).toHaveLength(1);
});

test("after reload, the same signed BW history renders the same status", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT_ENROLLED]: rootEvent(ROOT_ENROLLED, "Survives reload") },
      projection: { issues: { [ROOT_ENROLLED]: "backlog" } },
    },
  });
  const panel = await openIssuesPanel(page);
  const row = panel.locator(`[data-project-event-id="${ROOT_ENROLLED}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();
  await expect(panel.getByTestId("project-issue-status")).toContainText(
    "Backlog",
  );

  // `installMockBridge`'s seed is a `page.addInitScript`, so it re-applies
  // unchanged on reload — this proves the *read* path recomputes the same
  // projection from the same signed history on every replay (NIP-BW.md:
  // "Recompute from history on every replay"), not that this mock itself
  // persists a runtime write across a real native reload. The panel itself
  // isn't restored from URL state, so it's reopened explicitly post-reload.
  await page.reload();
  const panelAfterReload = await openIssuesPanel(page);
  const rowAfterReload = panelAfterReload.locator(
    `[data-project-event-id="${ROOT_ENROLLED}"]`,
  );
  await expect(rowAfterReload).toBeVisible({ timeout: 10_000 });
  await rowAfterReload.click();
  await expect(
    panelAfterReload.getByTestId("project-issue-status"),
  ).toContainText("Backlog");
});
