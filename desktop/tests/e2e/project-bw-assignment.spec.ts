import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// P4E: writer assignment over the *existing* kind:1 wire, the ready/
// in-development/implemented handoff, implemented-detail display and
// parent/child/blocks/duplicate-of relations. As in project-bw-issues.spec.ts,
// the mocked Tauri commands here perform no Core validation of their own —
// that authority is exclusively `buzz-core`/`buzz-sdk`'s job, proven against
// the pinned NIP-BW fixture corpus
// (crates/buzz-core/src/bw/{mod,semantics,projection}.rs,
// desktop/src-tauri/src/commands/{project_bw_assignment,project_bw_write}.rs).
// These specs prove the desktop UI calls the right command with the right
// shape and renders a real refusal or conflict visibly, never silently.

const POLICY_ID = "9".repeat(64);
const GENESIS_ID = "8".repeat(64);
const ROOT = "1".repeat(64);
const ROOT_B = "2".repeat(64);
const REPORTER = "c".repeat(64);
const WRITER = "d".repeat(64);
const OTHER_WRITER = "e".repeat(64);
const ASSIGN_A = "4".repeat(64);
const ASSIGN_B = "5".repeat(64);
const REPO_A = `30617:${"a".repeat(64)}:fixture-bw`;
const ACTIVATION = { policy: POLICY_ID, genesis: GENESIS_ID };

function rootEvent(id: string, subject: string) {
  return {
    id,
    pubkey: REPORTER,
    created_at: 1_800_000_000,
    kind: 1621,
    tags: [
      ["a", REPO_A],
      ["subject", subject],
    ],
    content: "Fixture description",
    sig: "0".repeat(128),
  };
}

function assignmentEvent(
  id: string,
  issue: string,
  delegate: string,
  operation: "assignment" | "unassignment",
  prior?: string,
) {
  const tags = [
    ["e", issue, "", "root"],
    ["a", REPO_A],
    ["p", delegate],
    ["t", operation],
  ];
  if (prior) tags.push(["prior", prior]);
  return {
    id,
    pubkey: WRITER,
    created_at: 1_800_000_200,
    kind: 1,
    tags,
    content: operation === "assignment" ? "Assigned" : "Unassigned",
    sig: "0".repeat(128),
  };
}

async function openIssuesPanel(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.getByTestId("channel-issues-trigger").click();
  const panel = page.getByTestId("channel-issues-auxiliary-pane");
  await expect(panel).toBeVisible();
  return panel;
}

async function openIssue(
  page: import("@playwright/test").Page,
  panel: ReturnType<typeof page.getByTestId>,
  issueId: string,
) {
  const row = panel.locator(`[data-project-event-id="${issueId}"]`);
  await expect(row).toBeVisible({ timeout: 10_000 });
  await row.click();
}

async function lastBwCall(
  page: import("@playwright/test").Page,
  command: string,
) {
  const calls = await page.evaluate(
    (cmd) =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) => entry.command === cmd,
      ),
    command,
  );
  // Every BW Tauri command takes a single `input` struct parameter
  // (`desktop/src-tauri/src/commands/{project_bw_write,project_bw_assignment}.rs`),
  // so real invoke args are always `{ input: {...} }`. Unwrap here so
  // callers can assert against the record fields directly.
  const call = calls.at(-1);
  if (!call) return call;
  return { ...call, payload: (call.payload as { input?: unknown })?.input };
}

test("a backlog issue with no current writer offers assign; assigning submits the existing kind:1 wire with no prior", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Needs a writer") },
      projection: { issues: { [ROOT]: "backlog" } },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  // The name/agent picker is the default path; raw hex is the secondary
  // fallback reached through this toggle (P4E follow-up: mention-style
  // picker instead of manual hex).
  await panel.getByTestId("bw-assign-toggle-manual").click();
  await expect(panel.getByTestId("bw-assign-delegate")).toBeVisible();
  await expect(panel.getByTestId("bw-assign")).toBeDisabled();
  await panel.getByTestId("bw-assign-delegate").fill(WRITER);
  await expect(panel.getByTestId("bw-assign")).toBeEnabled();
  await panel.getByTestId("bw-assign").click();

  const call = await lastBwCall(page, "submit_project_bw_assignment");
  expect(
    (call?.payload as {
      delegate?: string;
      operation?: string;
      prior?: unknown;
    }) ?? {},
  ).toMatchObject({ delegate: WRITER, operation: "assignment", prior: null });
});

test("assigning through the name picker submits the exact pubkey it resolved, same as the raw-hex fallback", async ({
  page,
}) => {
  const AGENT_NAME = "Windows Writer Agent";
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Needs a writer") },
      projection: { issues: { [ROOT]: "backlog" } },
    },
    relayAgents: [
      {
        pubkey: WRITER,
        name: AGENT_NAME,
        respondTo: "anyone",
        channelIds: [],
      },
    ],
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  // Default path: type a name, pick the relay-agent suggestion — no hex
  // typed anywhere.
  await expect(panel.getByTestId("bw-assign-picker-query")).toBeVisible();
  await panel.getByTestId("bw-assign-picker-query").fill("Windows Writer");
  const suggestion = page.getByTestId(`mention-suggestion-${WRITER}`);
  await expect(suggestion).toBeVisible();
  await suggestion.click();
  await expect(panel.getByTestId("bw-assign")).toBeEnabled();
  await panel.getByTestId("bw-assign").click();

  const call = await lastBwCall(page, "submit_project_bw_assignment");
  expect(call?.payload).toMatchObject({
    delegate: WRITER,
    operation: "assignment",
    prior: null,
  });
});

test("a backlog issue with a current writer offers unassign, resubmitting the exact same delegate and chaining off the real head", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT]: rootEvent(ROOT, "Already assigned"),
        [ASSIGN_A]: assignmentEvent(ASSIGN_A, ROOT, WRITER, "assignment"),
      },
      projection: { issues: { [ROOT]: "backlog" } },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  // The header now shows a display-name/avatar + truncated pubkey instead of
  // the naked 64-hex; the full pubkey is still verifiable via the `title`
  // attribute (truncatePubkey.ts: "never an identity proof" on its own).
  await expect(panel.getByTestId("bw-assignment-writer")).toHaveAttribute(
    "title",
    WRITER,
  );
  await panel.getByTestId("bw-unassign").click();

  const call = await lastBwCall(page, "submit_project_bw_assignment");
  expect(call?.payload).toMatchObject({
    delegate: WRITER,
    operation: "unassignment",
    prior: ASSIGN_A,
  });
});

// `bwAssignmentHead` only ever sees `snapshot.records`, and the real desktop
// bridge (`bw_projection.rs::snapshot`) only ever puts an *accepted* event
// there — a genuinely forked pair of kind:1 assignment candidates never both
// reach `accept`, so this exact `records` shape cannot occur against the
// real backend today (the same is true of the pre-existing `bwFieldConflict`
// check for 46100 forks). This still proves the frontend's own defensive
// contract: *if* two accepted heads were ever presented, the UI blocks
// further assignment actions rather than guessing a winner.
test("two accepted assignment heads (a belt-and-suspenders case bwAssignmentHead defends against) block further assignment actions", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT]: rootEvent(ROOT, "Forked assignment"),
        [ASSIGN_A]: assignmentEvent(ASSIGN_A, ROOT, WRITER, "assignment"),
        [ASSIGN_B]: assignmentEvent(ASSIGN_B, ROOT, OTHER_WRITER, "assignment"),
      },
      projection: { issues: { [ROOT]: "backlog" } },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await expect(panel.getByTestId("bw-assignment-conflict")).toBeVisible();
  await expect(panel.getByTestId("bw-assign")).toHaveCount(0);
  await expect(panel.getByTestId("bw-unassign")).toHaveCount(0);
});

// This is the signal a real forked assignment chain actually produces today:
// `bw_projection.rs::notice_issue` now covers kind:1 assignment/unassignment
// candidates the same way it already covered 46100 records, so a rejected or
// conflicted assignment event surfaces as a per-issue notice — visible,
// never silently dropped — even though it never appears in `records` itself.
test("a forked assignment candidate surfaces as a visible per-issue notice", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Forked assignment") },
      notices: {
        [ROOT]: [
          {
            event_id: ASSIGN_B,
            outcome: "conflict",
            stage: "causality",
            code: "assignment-operation",
          },
        ],
      },
      projection: { issues: { [ROOT]: "backlog" } },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await expect(
    panel.getByText("conflict: causality / assignment-operation"),
  ).toBeVisible();
});

test("an assignment refused by Core surfaces the refusal instead of a silent state change", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Wrong-role assign") },
      projection: { issues: { [ROOT]: "backlog" } },
    },
    bwAssignmentErrors: ["bw:reject:role:unauthorized"],
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await panel.getByTestId("bw-assign-toggle-manual").click();
  await panel.getByTestId("bw-assign-delegate").fill(WRITER);
  await panel.getByTestId("bw-assign").click();
  await expect(page.getByText("bw:reject:role:unauthorized")).toBeVisible();
});

test("backlog with a single-branch repo pre-fills the stream as a read-only field and submits it as-is", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Ready candidate") },
      projection: { issues: { [ROOT]: "backlog" } },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  // The mock repository has exactly one remote branch ("main"): it is
  // pre-filled and read-only, never an open dropdown or editable input.
  // `fill()` on a readonly field retries actionability until its own
  // timeout instead of failing fast, so non-editability is asserted via
  // the `readonly` attribute rather than an attempted write.
  const streamField = panel.getByTestId("bw-ready-stream");
  await expect(streamField).toHaveValue("main");
  await expect(streamField).toHaveAttribute("readonly", "");
  expect(await streamField.evaluate((el) => el.tagName)).toBe("INPUT");
  await expect(panel.getByTestId("bw-ready-submit")).toBeEnabled();
  await panel.getByTestId("bw-ready-submit").click();

  const call = await lastBwCall(page, "submit_project_bw_record");
  expect(call?.payload).toMatchObject({
    record: "issue-state",
    content: expect.objectContaining({
      state: "ready",
      stream: "main",
    }),
  });
});

test("ready offers starting development, reusing the ready head's own stream and assignment with no extra input", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Starting development") },
      projection: {
        issues: { [ROOT]: "ready" },
        issue_state: {
          [ROOT]: {
            state: "ready",
            stream: "windows-integration",
            assignment: ASSIGN_A,
          },
        },
        issue_state_id: { [ROOT]: "6".repeat(64) },
      },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await panel.getByTestId("bw-in-development-submit").click();
  const call = await lastBwCall(page, "submit_project_bw_record");
  expect(call?.payload).toMatchObject({
    record: "issue-state",
    content: {
      state: "in-development",
      stream: "windows-integration",
      assignment: ASSIGN_A,
    },
  });
});

test("in-development requires a tests summary and never sends a client-claimed commit or readback", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Marking implemented") },
      projection: {
        issues: { [ROOT]: "in-development" },
        issue_state: {
          [ROOT]: {
            state: "in-development",
            stream: "windows-integration",
            assignment: ASSIGN_A,
          },
        },
        issue_state_id: { [ROOT]: "6".repeat(64) },
      },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await expect(panel.getByTestId("bw-implemented-submit")).toBeDisabled();
  await panel.getByTestId("bw-implemented-tests").fill("cargo test: 0 failed");
  await expect(panel.getByTestId("bw-implemented-submit")).toBeEnabled();
  await panel.getByTestId("bw-implemented-submit").click();

  const call = await lastBwCall(page, "submit_project_bw_record");
  const content = (call?.payload as { content?: Record<string, unknown> })
    ?.content;
  expect(content).toMatchObject({
    state: "implemented",
    stream: "windows-integration",
    assignment: ASSIGN_A,
    tests: "cargo test: 0 failed",
  });
  // The externally observed commit/readback are resolved by the Tauri
  // command itself, never sent as a caller claim.
  expect(content).not.toHaveProperty("commit");
  expect(content).not.toHaveProperty("remote_readback");
});

test("an implemented issue displays the commit, tests and verified readback exactly as Core accepted them", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Already implemented") },
      projection: {
        issues: { [ROOT]: "implemented" },
        issue_state: {
          [ROOT]: {
            state: "implemented",
            stream: "windows-integration",
            assignment: ASSIGN_A,
            commit: "1111111111111111111111111111111111111111",
            tests: "cargo test: 0 failed",
            remote_readback: {
              repo: REPO_A,
              stream: "windows-integration",
              head: "1111111111111111111111111111111111111111",
              observed_at: 1_800_000_300,
            },
          },
        },
        issue_state_id: { [ROOT]: "6".repeat(64) },
      },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  const details = panel.getByTestId("bw-implemented-details");
  await expect(details).toContainText(
    "1111111111111111111111111111111111111111",
  );
  await expect(details).toContainText("cargo test: 0 failed");
  await expect(details).toContainText(
    "windows-integration@1111111111111111111111111111111111111111",
  );
  await expect(panel.getByTestId("bw-implemented-action")).toHaveCount(0);
});

test("relations show both directions and leaf eligibility, and adding/removing a direct edge submits an issue-relation record", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: {
        [ROOT]: rootEvent(ROOT, "Parent issue"),
        [ROOT_B]: rootEvent(ROOT_B, "Child issue"),
      },
      projection: {
        issues: { [ROOT]: "backlog", [ROOT_B]: "backlog" },
        leaf: { [ROOT]: false, [ROOT_B]: true },
        relations: [{ issue: ROOT, relation: "parent-of", target: ROOT_B }],
      },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await expect(panel.getByTestId("bw-leaf-state")).toContainText(
    "Not an executable leaf",
  );
  const row = panel.getByTestId("bw-relation-row");
  await expect(row).toContainText("parent-of");
  await expect(row).toContainText(ROOT_B);
  await row.getByTestId("bw-relation-remove").click();

  const removeCall = await lastBwCall(page, "submit_project_bw_record");
  expect(removeCall?.payload).toMatchObject({
    record: "issue-relation",
    content: { relation: "parent-of", target: ROOT_B, operation: "remove" },
  });

  await panel.getByTestId("bw-relation-type").selectOption("blocks");
  await panel.getByTestId("bw-relation-target").fill(ROOT_B);
  await expect(panel.getByTestId("bw-relation-add")).toBeEnabled();
  await panel.getByTestId("bw-relation-add").click();

  const addCall = await lastBwCall(page, "submit_project_bw_record");
  expect(addCall?.payload).toMatchObject({
    record: "issue-relation",
    content: { relation: "blocks", target: ROOT_B, operation: "add" },
  });
});

test("a leaf issue with no relations shows itself as executable", async ({
  page,
}) => {
  await installMockBridge(page, {
    bwSnapshot: {
      activation: ACTIVATION,
      records: { [ROOT]: rootEvent(ROOT, "Standalone issue") },
      projection: {
        issues: { [ROOT]: "backlog" },
        leaf: { [ROOT]: true },
      },
    },
  });
  const panel = await openIssuesPanel(page);
  await openIssue(page, panel, ROOT);

  await expect(panel.getByTestId("bw-leaf-state")).toContainText(
    "Executable leaf",
  );
  await expect(panel.getByTestId("bw-relation-row")).toHaveCount(0);
});
