/**
 * Reports tab behavior tests for AdminConsolePanel. Covers report detail
 * rendering (DTO cluster), processing-report navigation, community grouping,
 * reopen/resolve/cancel lifecycle, attachment budget, reason-audience
 * disclosure, frozen-payload retry, and canMutate gates.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  act,
  fireEvent,
  setIpcHandler,
  capturedToasts,
  capturedErrorToasts,
  resetTestState,
  mutationReject,
  mountPanel,
  makeOpenReportFixtures,
  settle,
  CM_ORIGIN,
  CM_PUBKEY,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";

afterEach(resetTestState);

// ── structured detail layouts ────────────────────────────────────────────────
//
// Table-driven cluster for ReportDetail DTO rendering. Four rows cover:
//   ordinary-nested-message — status, note, nested author/content (no deletion)
//   resolved-by-note        — populated resolvedBy and note fields
//   deleted-nested-message  — heading, content, deleted indicator (deletedAt set)
//   nullable-degradation    — all nullable fields null → em-dash, no message block
//
// Shared navigation helper reused by rows that need detail open.
// Mutation evidence per row is preserved inline.

/**
 * Navigate a mounted panel into its first report detail row.
 * Returns after the detail has settled.
 */
async function openFirstDetailRow(container) {
  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    await settle(30);
    return;
  }
  throw new Error("no navigable report row found in panel");
}

const REPORT_DTO_ROWS = [
  {
    name: "ordinary-nested-message",
    desc: "ReportDetail shows field layout, not raw JSON — ordinary nested message",
    pubkey: "5".repeat(64),
    item: {
      id: "00000000-0000-0000-0000-000000000099",
      communityId: "00000000-0000-0000-0000-000000000002",
      communityHost: "relay.example.com",
      reportEventId: "aabb",
      reporterPubkey: "ccdd",
      targetKind: "event",
      target: "eeff",
      reportType: "spam",
      status: "open",
      createdAt: "2024-06-01T12:00:00Z",
    },
    detail: (item) => ({
      ...item,
      channelId: "00000000-0000-0000-0000-000000000003",
      note: "private moderator note",
      resolvedBy: null,
      resolvedAt: null,
      actionId: null,
      message: {
        authorPubkey: "aabbccdd",
        content: "offensive message text",
        createdAt: "2024-05-31T10:00:00Z",
        deletedAt: null,
      },
    }),
    // Mutation: revert ReportFields → <pre>{JSON.stringify(...)}</pre> → red.
    check: (text) => {
      assert.ok(
        text.includes("open"),
        `status 'open' must appear in structured layout; text: ${text.slice(0, 400)}`,
      );
      assert.ok(
        !text.includes('"status": "open"'),
        `raw JSON must not render; text: ${text.slice(0, 400)}`,
      );
      assert.ok(
        text.includes("private moderator note"),
        `note must render; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        text.includes("offensive message text"),
        `nested message content must render; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        text.includes("aabbccdd"),
        `nested message authorPubkey must render; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        !text.includes("reason"),
        `invented 'reason' field must not render; text: ${text.slice(0, 400)}`,
      );
      assert.ok(
        !text.includes("moderationNote"),
        `invented 'moderationNote' field must not render; text: ${text.slice(0, 400)}`,
      );
    },
  },
  {
    name: "resolved-by-note",
    desc: "wrong key lookup makes resolvedBy invisible — mutation evidence",
    pubkey: "8".repeat(64),
    item: {
      id: "00000000-0000-0000-0000-000000000088",
      communityId: "00000000-0000-0000-0000-000000000002",
      communityHost: "relay.example.com",
      reportEventId: "rr01",
      reporterPubkey: "pp01",
      targetKind: "event",
      target: "tt01",
      reportType: "harassment",
      status: "resolved",
      createdAt: "2024-06-01T12:00:00Z",
    },
    detail: (item) => ({
      ...item,
      channelId: null,
      note: "case closed",
      resolvedBy: "moderator_pubkey_hex",
      resolvedAt: "2024-06-02T08:00:00Z",
      actionId: null,
      message: null,
    }),
    // Mutation: rename `resolvedBy` → `resolvedByX` in ReportFields → red.
    check: (text) => {
      assert.ok(
        text.includes("moderator_pubkey_hex"),
        `resolvedBy value must render via data.resolvedBy; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        text.includes("case closed"),
        `note value must render via data.note; text: ${text.slice(0, 600)}`,
      );
    },
  },
  {
    name: "deleted-nested-message",
    desc: "removing message block hides content and deleted indicator",
    pubkey: "9".repeat(64),
    item: {
      id: "00000000-0000-0000-0000-000000000099",
      communityId: "00000000-0000-0000-0000-000000000002",
      communityHost: "relay.example.com",
      reportEventId: "rr02",
      reporterPubkey: "pp02",
      targetKind: "event",
      target: "tt02",
      reportType: "spam",
      status: "open",
      createdAt: "2024-06-01T12:00:00Z",
    },
    detail: (item) => ({
      ...item,
      channelId: null,
      note: null,
      resolvedBy: null,
      resolvedAt: null,
      actionId: null,
      message: {
        authorPubkey: "msg_author_pubkey",
        content: "buy cheap meds at spamsite.example",
        createdAt: "2024-06-01T11:55:00Z",
        deletedAt: "2024-06-01T12:10:00Z",
      },
    }),
    // Mutation: remove `{data.message != null && ...}` block → content absent → red.
    check: (text) => {
      assert.ok(
        text.includes("buy cheap meds at spamsite.example"),
        `nested message content must render; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        text.includes("msg_author_pubkey"),
        `nested message authorPubkey must render; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        text.includes("Reported message"),
        `"Reported message" heading must render; text: ${text.slice(0, 600)}`,
      );
      // Mutation: remove `{data.message.deletedAt != null && ...}` → "(deleted)" absent → red.
      assert.ok(
        text.includes("(deleted)"),
        `deleted indicator must render when deletedAt non-null; text: ${text.slice(0, 600)}`,
      );
    },
  },
  {
    name: "nullable-degradation",
    desc: "report detail renders em-dash for absent nullable fields, no message block",
    pubkey: "7".repeat(64),
    item: {
      id: "00000000-0000-0000-0000-000000000077",
      communityId: "00000000-0000-0000-0000-000000000002",
      communityHost: "relay.example.com",
      reportEventId: "aabb",
      reporterPubkey: "ccdd",
      targetKind: "pubkey",
      target: "eeff",
      reportType: "nudity",
      status: "open",
      createdAt: "2024-06-01T12:00:00Z",
    },
    detail: (item) => ({
      ...item,
      channelId: null,
      note: null,
      resolvedBy: null,
      resolvedAt: null,
      actionId: null,
      message: null,
    }),
    // Mutation: remove `value != null` guard in DetailRow → em-dash breaks for undefined → red.
    check: (text) => {
      assert.ok(
        text.includes("—"),
        `em-dash must appear for null nullable fields; text: ${text.slice(0, 600)}`,
      );
      assert.ok(
        !text.includes("Reported message"),
        `nested message block must not render when message is null; text: ${text.slice(0, 600)}`,
      );
    },
  },
];

for (const row of REPORT_DTO_ROWS) {
  test(`report-dto-${row.name}: ${row.desc}`, async () => {
    const origin = "https://admin.example.com";
    const item = row.item;
    const detail = row.detail(item);

    setIpcHandler("admin_list_reports", () => Promise.resolve([item]));
    setIpcHandler("admin_get_report", () => Promise.resolve(detail));
    setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

    const { container, doRender, unmount } = mountPanel({
      origin,
      pubkey: row.pubkey,
    });
    await doRender();
    await settle(30);
    await openFirstDetailRow(container);

    const fields = container.querySelector(
      "[data-testid='report-detail-fields']",
    );
    assert.ok(
      fields !== null,
      `[${row.name}] report-detail-fields element must render`,
    );

    const text = container.textContent ?? "";
    row.check(text);

    await unmount();
  });
}

test("processing-report-navigable-suppresses-resolve-form: a processing report opens into detail, shows enforcement state, and hides the resolve form", async () => {
  // Thufir finding 4: processing rows must stay navigable. The enforcement
  // state (progress/retry/cancel) lives inside the detail view, so disabling
  // the row hides exactly the UI an operator needs while an action is pending.
  // "Not actionable" means suppress the resolve form, not block navigation.
  //
  // Mutation evidence: re-add `disabled={isProcessing}` to the ReportsTab row →
  // the click never opens detail, report-detail-fields never renders → red.
  // Drop the `isOpen` gate on ResolveReportForm → the resolve form renders for
  // a processing report → the resolve-form-absent assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "f5".repeat(32);

  const processingItem = {
    id: "00000000-0000-0000-0000-000000000010",
    communityId: "00000000-0000-0000-0000-000000000002",
    communityHost: "relay.example.com",
    reportEventId: "aabb",
    reporterPubkey: "ccdd",
    targetKind: "event",
    target: "eeff",
    reportType: "spam",
    status: "processing",
    createdAt: "2024-01-01T00:00:00Z",
  };
  const processingDetail = {
    ...processingItem,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000e1",
      requestId: "00000000-0000-0000-0000-0000000000e2",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "enforcing",
      reason: null,
      expiresAt: null,
      errorMessage: null,
      createdAt: "2024-01-01T00:00:00Z",
      updatedAt: "2024-01-01T00:00:01Z",
    },
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([processingItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(processingDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // The processing row must be a navigable (non-disabled) button.
  const rowButtons = Array.from(container.querySelectorAll("button")).filter(
    (btn) => !(btn.getAttribute("data-testid") ?? "").startsWith("admin-tab"),
  );
  const processingRow = rowButtons.find((btn) =>
    btn.textContent?.includes("spam"),
  );
  assert.ok(processingRow, "processing report row must be present");
  assert.ok(
    !processingRow.disabled,
    "processing report row must stay navigable (not disabled)",
  );

  // Navigate into the detail.
  await act(async () => {
    fireEvent.click(processingRow);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // Detail renders (navigation succeeded).
  assert.ok(
    container.querySelector("[data-testid='report-detail-fields']"),
    "report-detail-fields must render after navigating into a processing report",
  );
  // Enforcement state block is shown for a processing report with an action.
  assert.ok(
    container.querySelector("[data-testid='enforcement-state-block']"),
    "enforcement-state-block must render for a processing report",
  );
  // The resolve form must be suppressed for a non-open (processing) report.
  assert.equal(
    container.querySelector("[data-testid='resolve-report-form']"),
    null,
    "resolve form must NOT render for a processing report",
  );

  await unmount();
});

// ── community grouping ────────────────────────────────────────────────────

test("reports-grouped-by-community: multi-community reports render per-community headings", async () => {
  // The admin API returns deployment-wide reports; the console buckets them
  // by community for triage. Two communities → two group headings; rows stay
  // navigable (the first non-tab, non-processing report opens its detail).
  //
  // Mutation evidence: revert ReportsTab to a flat <ul> → community-group
  // headings vanish and this test goes red.

  const origin = "https://admin.example.com";
  const pubkey = "a7".repeat(32);

  const reports = [
    {
      id: "00000000-0000-0000-0000-0000000000a1",
      communityId: "comm-1",
      communityHost: "alpha.example.com",
      reportEventId: "aa",
      reporterPubkey: "bb",
      targetKind: "event",
      target: "cc",
      reportType: "spam",
      status: "open",
      createdAt: "2024-06-01T12:00:00Z",
    },
    {
      id: "00000000-0000-0000-0000-0000000000a2",
      communityId: "comm-2",
      communityHost: "beta.example.com",
      reportEventId: "dd",
      reporterPubkey: "ee",
      targetKind: "event",
      target: "ff",
      reportType: "abuse",
      status: "open",
      createdAt: "2024-06-02T12:00:00Z",
    },
  ];

  setIpcHandler("admin_list_reports", () => Promise.resolve(reports));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  const groups = container.querySelectorAll("[data-testid='community-group']");
  assert.equal(
    groups.length,
    2,
    `two communities must render two groups; got ${groups.length}`,
  );

  const hosts = Array.from(
    container.querySelectorAll("[data-testid='community-group-host']"),
  ).map((el) => el.textContent);
  assert.deepEqual(
    hosts,
    ["alpha.example.com", "beta.example.com"],
    `group headings must show each community host in first-seen order; got: ${JSON.stringify(hosts)}`,
  );

  await unmount();
});

// ── reopen ────────────────────────────────────────────────────────────────

/**
 * Mount the panel, wait for the list, then click the first non-tab report row
 * to open its detail. Returns after the detail has settled.
 */
async function openFirstReportDetail(container) {
  const allButtons = container.querySelectorAll("button");
  for (const btn of allButtons) {
    const testid = btn.getAttribute("data-testid") ?? "";
    if (testid.startsWith("admin-tab")) continue;
    await act(async () => {
      fireEvent.click(btn);
      await new Promise((r) => setTimeout(r, 30));
    });
    return;
  }
  throw new Error("no navigable report row found");
}

test("reopen-form-gated-by-status: resolved report shows the reopen form", async () => {
  // The reopen form must render for terminal reports (resolved | dismissed |
  // escalated). This fixture uses a resolved report. The open-report half of
  // the gate (showing resolve form, no reopen form) is separately exercised by
  // reopen-submit and the resolve-path tests.
  //
  // Mutation evidence: drop the `isReopenable` gate → the reopen form renders
  // for open reports too and suppression logic is broken.

  const origin = "https://admin.example.com";
  const pubkey = "c1".repeat(32);

  const resolvedItem = {
    id: "00000000-0000-0000-0000-0000000000c1",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "resolved",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const resolvedDetail = {
    ...resolvedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([resolvedItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(resolvedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='reopen-report-form']"),
    "reopen form must render for a resolved report",
  );
  assert.equal(
    container.querySelector("[data-testid='resolve-report-form']"),
    null,
    "resolve form must NOT render for a resolved report",
  );

  await unmount();
});

test("reopen-submit: calls admin_reopen_report with requestId+reason, toasts, and refreshes", async () => {
  // The reopen submit must POST {requestId, reason} to admin_reopen_report,
  // fire a success toast, and bump the resolve generation so the detail
  // reloads (verified here by a second admin_get_report call returning the
  // now-open report, which flips the UI to the resolve form).
  //
  // Mutation evidence: remove `onReopened()` → no reload, detail stays
  // resolved, and the resolve-form assertion goes red. Remove the toast →
  // capturedToasts assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "c2".repeat(32);

  const base = {
    id: "00000000-0000-0000-0000-0000000000c2",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const dismissedItem = { ...base, status: "dismissed" };
  const dismissedDetail = {
    ...dismissedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: null,
    message: null,
  };
  const openDetail = {
    ...base,
    status: "open",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([dismissedItem]));
  // First detail load: dismissed. After reopen, the generation bump reloads
  // and the report is now open.
  let detailCalls = 0;
  setIpcHandler("admin_get_report", () => {
    detailCalls += 1;
    return Promise.resolve(detailCalls === 1 ? dismissedDetail : openDetail);
  });
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  let reopenArgs = null;
  setIpcHandler("admin_reopen_report", (args) => {
    reopenArgs = args;
    return Promise.resolve({ status: "open" });
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // Type a reason.
  const reasonInput = container.querySelector(
    "[data-testid='reopen-reason-input']",
  );
  assert.ok(reasonInput, "reopen reason input must be present");
  await act(async () => {
    fireEvent.change(reasonInput, { target: { value: "new evidence" } });
    await new Promise((r) => setTimeout(r, 5));
  });

  // Submit.
  const submit = container.querySelector("[data-testid='reopen-submit-btn']");
  assert.ok(submit, "reopen submit button must be present");
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(reopenArgs, "admin_reopen_report must be invoked");
  assert.equal(reopenArgs.origin, origin, "origin must be forwarded");
  assert.equal(reopenArgs.id, base.id, "report id must be forwarded");
  assert.equal(
    reopenArgs.body?.reason,
    "new evidence",
    "reason must be forwarded in the body",
  );
  assert.ok(
    typeof reopenArgs.body?.requestId === "string" &&
      reopenArgs.body.requestId.length > 0,
    `requestId must be a non-empty string; got: ${JSON.stringify(reopenArgs.body?.requestId)}`,
  );

  assert.ok(
    capturedToasts.some((m) => m.toLowerCase().includes("reopen")),
    `a reopen success toast must fire; got: ${JSON.stringify(capturedToasts)}`,
  );

  // Refresh: detail reloaded (call 2) and the report is now open → resolve form.
  assert.ok(
    detailCalls >= 2,
    "detail must reload after reopen (generation bump)",
  );
  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "after reopen, the now-open report must show the resolve form",
  );

  await unmount();
});

test("reopen-enforced-copy: a report with an actionId warns enforcement is not reversed", async () => {
  // Reopen is re-triage only. When the report carries an actionId (enforcement
  // was applied), the copy must say the enforcement is not reversed.
  //
  // Mutation evidence: collapse the `wasEnforced` branch to the generic copy →
  // the "not reversed" wording for un-ban/un-timeout/restore disappears.

  const origin = "https://admin.example.com";
  const pubkey = "c3".repeat(32);

  const escalatedItem = {
    id: "00000000-0000-0000-0000-0000000000c3",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "pubkey",
    target: "cc",
    reportType: "abuse",
    status: "escalated",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const escalatedDetail = {
    ...escalatedItem,
    channelId: null,
    note: null,
    resolvedBy: "mod_pubkey",
    resolvedAt: "2024-06-02T08:00:00Z",
    actionId: "00000000-0000-0000-0000-0000000000ff",
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([escalatedItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(escalatedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  const form = container.querySelector("[data-testid='reopen-report-form']");
  assert.ok(form, "reopen form must render for an escalated report");
  const text = form.textContent ?? "";
  assert.ok(
    text.toLowerCase().includes("not reversed"),
    `enforced-report copy must state the action is not reversed; got: ${text}`,
  );

  await unmount();
});

// Reopen retry idempotency — table-driven (4 rows)
//
// preserveRequestIdOnError semantics: the requestId must survive retries
// where the relay may have committed and the response was lost or ambiguous
// (409, null-status transport failure, incomplete 4xx body). A fresh requestId
// is only correct for a definitive pre-commit rejection (complete 4xx body).
//
// Each row mounts a resolved report, attempts reopen twice, and asserts
// whether the two requestIds are equal (preserved) or different (reset).
// Row-specific notes:
//   409 — relay claims ownership; a no-op retry prevents double-reopening.
//         Also asserts error toast present and no success toast.
//   null-status — no relay verdict at all (timeout/disconnect); must preserve.
//   complete-400 — full body read, definitive rejection; reset is safe.
//   truncated-400 — status arrived but body lost (bodyComplete: false); must
//                   preserve despite having a status code.
//
// Mutation evidence per row:
//   409: reset on 409 → different ids, RED.
//   null-status: reset on null → different ids, RED.
//   complete-400: preserve on 400 → same ids, RED.
//   truncated-400: reset every non-409 4xx → different ids, RED.
const REOPEN_RETRY_ROWS = [
  {
    name: "409",
    pubkey: "c4".repeat(32),
    id: "00000000-0000-0000-0000-0000000000c4",
    makeError: () =>
      mutationReject(
        "admin API error: 409 report is not reopenable (current status: processing)",
        409,
      ),
    preserved: true,
    checkToasts: (captured, capturedError) => {
      assert.ok(
        !captured.some((m) => m.toLowerCase().includes("reopen")),
        `no success toast on a 409; got: ${JSON.stringify(captured)}`,
      );
      assert.ok(
        capturedError.some((m) => m.includes("not reopenable")),
        `409 error must surface via toast.error; got: ${JSON.stringify(capturedError)}`,
      );
    },
  },
  {
    name: "null-status lost response",
    pubkey: "c5".repeat(32),
    id: "00000000-0000-0000-0000-0000000000c5",
    makeError: () => mutationReject("relay unreachable: network error", null),
    preserved: true,
  },
  {
    name: "complete-400 reset",
    pubkey: "c6".repeat(32),
    id: "00000000-0000-0000-0000-0000000000c6",
    makeError: () => mutationReject("admin API error: bad request", 400),
    preserved: false,
  },
  {
    name: "truncated-400 preserve",
    pubkey: "c9".repeat(32),
    id: "00000000-0000-0000-0000-0000000000c9",
    makeError: () =>
      mutationReject(
        "admin response stream error: connection reset",
        400,
        false,
      ),
    preserved: true,
  },
];

for (const row of REOPEN_RETRY_ROWS) {
  test(`reopen-retry-${row.name}: reopen requestId is ${row.preserved ? "preserved" : "reset"} on ${row.name}`, async () => {
    const origin = "https://admin.example.com";
    const { pubkey, id } = row;

    const resolvedItem = {
      id,
      communityId: "comm-1",
      communityHost: "alpha.example.com",
      reportEventId: "aa",
      reporterPubkey: "bb",
      targetKind: "event",
      target: "cc",
      reportType: "spam",
      status: "resolved",
      createdAt: "2024-06-01T12:00:00Z",
    };
    const resolvedDetail = {
      ...resolvedItem,
      channelId: null,
      note: null,
      resolvedBy: "mod_pubkey",
      resolvedAt: "2024-06-02T08:00:00Z",
      actionId: null,
      message: null,
    };

    setIpcHandler("admin_list_reports", () => Promise.resolve([resolvedItem]));
    setIpcHandler("admin_get_report", () => Promise.resolve(resolvedDetail));
    setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

    const requestIds = [];
    setIpcHandler("admin_reopen_report", (args) => {
      requestIds.push(args?.body?.requestId);
      return row.makeError();
    });

    const { container, doRender, unmount } = mountPanel({ origin, pubkey });
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    const submit = container.querySelector("[data-testid='reopen-submit-btn']");
    assert.ok(submit, `[${row.name}] reopen submit button must be present`);

    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });
    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(
      requestIds.length,
      2,
      `[${row.name}] two reopen attempts must have been made`,
    );
    if (row.preserved) {
      assert.equal(
        requestIds[0],
        requestIds[1],
        `[${row.name}] requestId must be preserved on retry; got: ${JSON.stringify(requestIds)}`,
      );
    } else {
      assert.notEqual(
        requestIds[0],
        requestIds[1],
        `[${row.name}] requestId must be reset after definitive rejection; got: ${JSON.stringify(requestIds)}`,
      );
    }

    if (row.checkToasts) {
      row.checkToasts(capturedToasts, capturedErrorToasts);
    }

    await unmount();
  });
}

// ── Report status fixture builder ─────────────────────────────────────────────
//
// Builds the invariant base object shared by the cancel/no-cancel/reopened-
// after-enforcement trio. Reload handlers, action objects, IDs, counters, and
// per-test assertions stay test-local.

function makeReportBase({
  id,
  communityId = "00000000-0000-0000-0000-000000000002",
  communityHost = "relay.example.com",
  reportEventId = "aa",
  reporterPubkey = "bb",
  targetKind = "event",
  target = "cc",
  reportType = "spam",
  createdAt = "2024-06-01T12:00:00Z",
}) {
  return {
    id,
    communityId,
    communityHost,
    reportEventId,
    reporterPubkey,
    targetKind,
    target,
    reportType,
    createdAt,
  };
}

test("cancel-on-failed: a failed action offers Cancel, POSTs {actionId} to admin_cancel_report, and reloads to open", async () => {
  // Cancel-then-resolve is the only recovery from a failed enforcement. The
  // block offers Cancel on `status: "failed"`, fences it on the action id, and
  // on success the report returns to `open` — the detail reload then serves
  // activeAction: null and re-exposes the resolve form for a fresh attempt.
  //
  // Mutation evidence: revert handleCancel to the old resolve-with-dismiss
  // masquerade → admin_cancel_report is never called and cancelArgs stays null.
  // Restore the `!activeAction` gate on the resolve form → the reopened report
  // still carries no action here, so this test isolates the cancel wiring.

  const origin = "https://admin.example.com";
  const pubkey = "e5".repeat(32);

  const base = makeReportBase({ id: "00000000-0000-0000-0000-0000000000e5" });
  const actionId = "00000000-0000-0000-0000-0000000000f1";
  const failedDetail = {
    ...base,
    status: "processing",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: actionId,
      requestId: "00000000-0000-0000-0000-0000000000f2",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "failed",
      reason: null,
      expiresAt: null,
      errorMessage: "adapter timeout",
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:05Z",
    },
    message: null,
  };
  const openDetail = {
    ...base,
    status: "open",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "processing" }]),
  );
  let detailCalls = 0;
  setIpcHandler("admin_get_report", () => {
    detailCalls += 1;
    return Promise.resolve(detailCalls === 1 ? failedDetail : openDetail);
  });
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  let cancelArgs = null;
  setIpcHandler("admin_cancel_report", (args) => {
    cancelArgs = args;
    return Promise.resolve({
      status: "open",
      activeAction: { ...failedDetail.activeAction, status: "cancelled" },
    });
  });
  // The dismiss-masquerade path must be gone: resolve must never be called.
  let resolveCalled = false;
  setIpcHandler("admin_resolve_report", () => {
    resolveCalled = true;
    return Promise.reject(new Error("resolve must not be called by cancel"));
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // The failed action surfaces the error message and a single Cancel button.
  const block = container.querySelector(
    "[data-testid='enforcement-state-block']",
  );
  assert.ok(block, "enforcement-state-block must render for a failed action");
  assert.ok(
    (block.textContent ?? "").includes("adapter timeout"),
    `the failure errorMessage must render; got: ${block.textContent}`,
  );
  assert.equal(
    container.querySelector("[data-testid='enforcement-retry-btn']"),
    null,
    "the composed-retry button must be gone (Cancel-only on failed)",
  );
  const cancelBtn = container.querySelector(
    "[data-testid='enforcement-cancel-btn']",
  );
  assert.ok(cancelBtn, "the Cancel button must render on a failed action");

  await act(async () => {
    fireEvent.click(cancelBtn);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(cancelArgs, "admin_cancel_report must be invoked");
  assert.equal(cancelArgs.origin, origin, "origin must be forwarded");
  assert.equal(cancelArgs.id, base.id, "report id must be forwarded");
  assert.equal(
    cancelArgs.body?.actionId,
    actionId,
    "cancel must be fenced on the observed action id",
  );
  assert.equal(
    resolveCalled,
    false,
    "cancel must NOT go through the resolve endpoint (no dismiss masquerade)",
  );
  assert.ok(
    capturedToasts.some((m) => m.toLowerCase().includes("cancel")),
    `a cancel success toast must fire; got: ${JSON.stringify(capturedToasts)}`,
  );
  // Detail reloaded; the now-open report shows the resolve form for re-triage.
  assert.ok(detailCalls >= 2, "detail must reload after cancel");
  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "after cancel the reopened report must show the resolve form",
  );

  await unmount();
});

test("no-cancel-on-in-flight: an enforcing action offers no cancel button", async () => {
  // Only a pre-mutation `failed` action is cancellable over HTTP. An
  // `enforcing` action is owned by the relay's recovery worker; the UI must
  // not offer a button that 409s by design.
  //
  // This fixture exercises the enforcing state. The pending state is not
  // separately exercised here; the gate is the same `=== "failed"` check.
  //
  // Mutation evidence: change the button gate from `=== "failed"` to include
  // enforcing → the assertion that no cancel button renders goes red.

  const origin = "https://admin.example.com";
  const pubkey = "e6".repeat(32);

  const base = makeReportBase({ id: "00000000-0000-0000-0000-0000000000e6" });
  const enforcingDetail = {
    ...base,
    status: "processing",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000f3",
      requestId: "00000000-0000-0000-0000-0000000000f4",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "enforcing",
      reason: null,
      expiresAt: null,
      errorMessage: null,
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:01Z",
    },
    message: null,
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "processing" }]),
  );
  setIpcHandler("admin_get_report", () => Promise.resolve(enforcingDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='enforcement-state-block']"),
    "enforcement-state-block must render for an enforcing action",
  );
  assert.equal(
    container.querySelector("[data-testid='enforcement-cancel-btn']"),
    null,
    "no cancel button on an in-flight (enforcing) action",
  );
  // And the resolve form must stay suppressed on a processing report.
  assert.equal(
    container.querySelector("[data-testid='resolve-report-form']"),
    null,
    "resolve form must not render on a processing report",
  );

  await unmount();
});

test("reopened-after-enforcement: an open report carrying a succeeded action shows both history and the resolve form", async () => {
  // Honest history: a report enforced then reopened is `open` yet the detail
  // LATERAL still returns the succeeded action (the ban actually ran — a later
  // reopen does not un-happen it). The UI must render that action as executed
  // history AND still offer the resolve form, because the report is open for
  // re-triage. Cancel must NOT appear — cancel is failed-only.
  //
  // Mutation evidence: restore the `isOpen && !activeAction` gate → the resolve
  // form vanishes on this report and the operator is stranded, going red.

  const origin = "https://admin.example.com";
  const pubkey = "e7".repeat(32);

  const base = makeReportBase({ id: "00000000-0000-0000-0000-0000000000e7" });
  const reopenedDetail = {
    ...base,
    status: "open",
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000f5",
      requestId: "00000000-0000-0000-0000-0000000000f6",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "ban",
      status: "succeeded",
      reason: "confirmed spam",
      expiresAt: null,
      errorMessage: null,
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:03Z",
    },
    message: null,
    createdAt: "2024-06-01T11:00:00Z",
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...reopenedDetail, activeAction: undefined }]),
  );
  setIpcHandler("admin_get_report", () => Promise.resolve(reopenedDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // Executed-enforcement history renders.
  const block = container.querySelector(
    "[data-testid='enforcement-state-block']",
  );
  assert.ok(block, "the succeeded action must render as enforcement history");
  assert.ok(
    (block.textContent ?? "").toLowerCase().includes("succeeded"),
    `history must show the succeeded state; got: ${block.textContent}`,
  );
  // Cancel is failed-only — never on a succeeded action.
  assert.equal(
    container.querySelector("[data-testid='enforcement-cancel-btn']"),
    null,
    "no cancel button on a succeeded action",
  );
  // The resolve form must still show — the report is open for re-triage.
  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "an open reopened-after-enforcement report must still show the resolve form",
  );

  await unmount();
});

// ── D3a: kick suppressed when the report carries no channel ────────────────

test("kick-suppressed-when-channel-null: an event report without a channel hides the Kick action", async () => {
  // Kick removes the target from the report's associated channel, so the relay
  // 400s (invalid_action_for_target) when the report has no channelId. The
  // resolve form must not offer an action guaranteed to fail. Other event
  // actions (ban/timeout/dismiss/delete/escalate) stay available.
  //
  // Mutation evidence: drop the `.filter((a) => a !== "kick" || channelId
  // != null)` guard → action-btn-kick renders and the null-channel assertion
  // goes red.

  const origin = "https://admin.example.com";
  const pubkey = "d3".repeat(32);

  const item = {
    id: "00000000-0000-0000-0000-0000000000d3",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const detail = {
    ...item,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([item]));
  setIpcHandler("admin_get_report", () => Promise.resolve(detail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  assert.ok(
    container.querySelector("[data-testid='resolve-report-form']"),
    "resolve form must render for an open report",
  );
  assert.equal(
    container.querySelector("[data-testid='action-btn-kick']"),
    null,
    "Kick must be suppressed when the report has no channelId",
  );
  // Sibling event actions remain available — only Kick is gated.
  assert.ok(
    container.querySelector("[data-testid='action-btn-ban']"),
    "Ban must still be offered on an event report",
  );

  await unmount();
});

// ── D2: lists refetch on back-nav after a mutation ─────────────────────────

test("reports-list-refetches-on-back-after-mutation: resolving a report then navigating back shows fresh list status", async () => {
  // A mutation in the detail bumps a list generation fence propagated to the
  // ReportsTab, so returning to the list refetches instead of serving the
  // stale cached rows (Will's tab-switch workaround). Evidence is a second
  // admin_list_reports call after back-nav returning the updated status.
  //
  // Mutation evidence: drop the onMutated → setListGen wiring → the list
  // query key never changes, admin_list_reports is called once, and the
  // second-call assertion goes red.

  const origin = "https://admin.example.com";
  const pubkey = "d5".repeat(32);

  const openItem = {
    id: "00000000-0000-0000-0000-0000000000d5",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "pubkey",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const openDetail = {
    ...openItem,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  // The list returns "open" first, then "dismissed" after the mutation — the
  // refetch must surface the new status.
  let listCalls = 0;
  setIpcHandler("admin_list_reports", () => {
    listCalls += 1;
    return Promise.resolve([
      { ...openItem, status: listCalls === 1 ? "open" : "dismissed" },
    ]);
  });
  setIpcHandler("admin_get_report", () => Promise.resolve(openDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_resolve_report", () =>
    Promise.resolve({ status: "dismissed" }),
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  await openFirstReportDetail(container);
  await settle(20);
  const callsBeforeBack = listCalls;

  // Dismiss the report.
  const dismissBtn = container.querySelector(
    "[data-testid='action-btn-dismiss']",
  );
  assert.ok(dismissBtn, "dismiss action must be present");
  await act(async () => {
    fireEvent.click(dismissBtn);
    await new Promise((r) => setTimeout(r, 10));
  });
  const submit = container.querySelector("[data-testid='resolve-submit-btn']");
  assert.ok(
    submit,
    "resolve submit button must appear after selecting dismiss",
  );
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  // Navigate back to the list.
  const backBtn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Back to reports"),
  );
  assert.ok(backBtn, "back-to-reports button must be present");
  await act(async () => {
    fireEvent.click(backBtn);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  assert.ok(
    listCalls > callsBeforeBack,
    `the list must refetch after back-nav following a mutation; before=${callsBeforeBack} after=${listCalls}`,
  );
  assert.ok(
    (container.textContent ?? "").includes("dismissed"),
    `the refetched list must show the updated status; got: ${(container.textContent ?? "").slice(0, 400)}`,
  );

  await unmount();
});

test("resolve-rejection-surfaces-parsed-message: a rejected resolve toasts the relay message, not raw JSON", async () => {
  // A resolve mutation that the relay rejects (e.g. invalid_action_for_target)
  // must surface the envelope's human message via toast.error — never the raw
  // JSON envelope and never a success toast.
  //
  // Mutation evidence: replace `toast.error(adminErrorMessage(e))` in
  // handleSubmit with `toast.error(String(e))` → the raw-JSON assertion goes
  // red because the envelope leaks verbatim.

  const origin = "https://admin.example.com";
  const pubkey = "f7".repeat(32);

  const openItem = {
    id: "00000000-0000-0000-0000-0000000000f7",
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "aa",
    reporterPubkey: "bb",
    targetKind: "event",
    target: "cc",
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const openDetail = {
    ...openItem,
    channelId: "00000000-0000-0000-0000-0000000000ff",
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };

  const humanMessage =
    "action kick requires the report to have an associated channel";
  // The native command rejects with a typed AdminMutationError: message is
  // `admin API error: {envelope}` (the shape adminErrorMessage strips to the
  // envelope's `message`) and relayStatus is the relay's 400.
  const rawError = `admin API error: {"error":{"code":"invalid_action_for_target","message":"${humanMessage}","requestId":"00000000-0000-0000-0000-0000000000e7"}}`;

  setIpcHandler("admin_list_reports", () => Promise.resolve([openItem]));
  setIpcHandler("admin_get_report", () => Promise.resolve(openDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_resolve_report", () => mutationReject(rawError, 400));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);
  await openFirstReportDetail(container);
  await settle(20);

  // Select the kick action, then submit — the relay rejects it.
  const kickBtn = container.querySelector("[data-testid='action-btn-kick']");
  assert.ok(kickBtn, "kick action must be present (channel is set)");
  await act(async () => {
    fireEvent.click(kickBtn);
    await new Promise((r) => setTimeout(r, 10));
  });
  const submit = container.querySelector("[data-testid='resolve-submit-btn']");
  assert.ok(submit, "resolve submit button must appear after selecting kick");
  await act(async () => {
    fireEvent.click(submit);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(20);

  // The parsed human message reaches toast.error.
  assert.ok(
    capturedErrorToasts.some((m) => m.includes(humanMessage)),
    `the parsed relay message must surface via toast.error; got: ${JSON.stringify(capturedErrorToasts)}`,
  );
  // The raw JSON envelope must NOT leak into any error toast.
  assert.ok(
    !capturedErrorToasts.some(
      (m) => m.includes('{"error"') || m.includes("admin API error:"),
    ),
    `the raw JSON envelope must not appear in an error toast; got: ${JSON.stringify(capturedErrorToasts)}`,
  );
  // No success toast on a rejected resolve.
  assert.ok(
    !capturedToasts.some((m) => m.toLowerCase().includes("resolved")),
    `no success toast on a rejected resolve; got: ${JSON.stringify(capturedToasts)}`,
  );

  await unmount();
});

// ── P1-2: attachment budget enforced at the component seam ─────────────────
//
// Carl finding P1-2: the regression must prove excess attachments are NEVER
// requested, not just that the pure helper truncates them. The test renders
// FeedbackDetail with 7 image imeta entries, counts native IPC calls, and
// asserts that exactly 5 hashes are requested and 2 are never seen.
//
// Fails if `applyAttachmentBudget` is bypassed at AdminConsoleFeedbackTab.tsx
// (e.g. by mapping `allAttachments` directly instead of the `shown` slice).

test("attachment-budget-seam: only 5 of 7 image attachments trigger native fetch", async () => {
  const origin = "https://admin.example.com";
  const pubkey = "ab".repeat(32);

  // Build 7 distinct image attachments — sha256s are deterministic so we can
  // assert which hashes were and were not requested.
  const makeAttachment = (n) => {
    const sha = String(n).repeat(64).slice(0, 64);
    return {
      sha256: sha,
      mime: "image/png",
      size: 1024,
      url: `https://relay.example.com/files/${sha}`,
    };
  };
  const attachments = [0, 1, 2, 3, 4, 5, 6].map(makeAttachment);

  const feedbackId = "00000000-0000-0000-0000-000000000077";
  const summary = {
    id: feedbackId,
    communityId: "comm-budget",
    communityHost: "relay.example.com",
    submitterPubkey: "submitter-budget",
    category: null,
    bodySummary: "Budget test feedback",
    receivedAt: "2024-01-01T00:00:00Z",
  };
  const detail = {
    id: feedbackId,
    communityId: "comm-budget",
    communityHost: "relay.example.com",
    eventId: "budgetevent",
    submitterPubkey: "submitter-budget",
    category: null,
    body: "Budget test feedback full body",
    status: "new",
    tags: attachments.map((a) => [
      "imeta",
      `url ${a.url}`,
      `m ${a.mime}`,
      `x ${a.sha256}`,
      `size ${a.size}`,
    ]),
    eventCreatedAt: "2024-01-01T00:00:00Z",
    receivedAt: "2024-01-01T00:00:00Z",
  };

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([summary]));
  setIpcHandler("admin_get_feedback", () => Promise.resolve(detail));

  // Track every sha256 that is actually requested via the native IPC command.
  const requestedSha256s = [];
  if (!globalThis.URL) globalThis.URL = {};
  globalThis.URL.createObjectURL = () => "blob:test-budget";
  globalThis.URL.revokeObjectURL = () => {};
  setIpcHandler("admin_fetch_feedback_attachment", (args) => {
    requestedSha256s.push(args?.sha256);
    // Return a minimal ArrayBuffer so fetchAdminAttachmentBlobUrl can create a
    // Blob and call URL.createObjectURL without throwing.
    return Promise.resolve(new Uint8Array([1, 2, 3]).buffer);
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  // Navigate to the Feedback tab.
  const feedbackTab = container.querySelector(
    "[data-testid='admin-tab-feedback']",
  );
  assert.ok(feedbackTab, "Feedback tab must be present");
  await act(async () => {
    fireEvent.click(feedbackTab);
    await new Promise((r) => setTimeout(r, 30));
  });
  await settle(30);

  // Click the feedback list item to open detail — the first non-tab button.
  const listButtons = Array.from(container.querySelectorAll("button")).filter(
    (b) => !(b.getAttribute("data-testid") ?? "").startsWith("admin-tab"),
  );
  assert.ok(
    listButtons.length > 0,
    "feedback list item button must be present",
  );
  await act(async () => {
    fireEvent.click(listButtons[0]);
    await new Promise((r) => setTimeout(r, 50));
  });
  await settle(50);

  // After detail loads, all 7 AttachmentViewers would mount if the budget were
  // bypassed — each auto-loads image/* on mount. With the budget in place only
  // 5 mount and issue fetches.
  try {
    assert.equal(
      requestedSha256s.length,
      5,
      `exactly 5 attachment fetches must fire; got ${requestedSha256s.length}: ${JSON.stringify(requestedSha256s)}`,
    );

    // The 6th and 7th items (sha256 of attachments[5] and attachments[6]) must
    // never appear in the fetch log — the budget silently drops them.
    const excessHashes = [attachments[5].sha256, attachments[6].sha256];
    for (const excess of excessHashes) {
      assert.ok(
        !requestedSha256s.includes(excess),
        `excess attachment sha256 ${excess.slice(0, 8)}… must never be requested (budget bypass detected)`,
      );
    }

    // Truncation notice must be visible.
    const notice = container.querySelector(
      "[data-testid='attachment-truncated-notice']",
    );
    assert.ok(
      notice !== null,
      "truncation notice must render when attachments are capped",
    );
  } finally {
    await unmount();
  }
});

// ── P2-1: canMutate gates every mutation affordance ────────────────────────
//
// Carl finding P2-1: "every mutation affordance in the panel" must be gated
// on canMutate. Families covered:
//   A. Report resolve form (open report → ResolveReportForm)
//   B. Report reopen form (resolved report → ReopenReportForm)
//   C. Enforcement cancel button (failed activeAction → EnforcementStateBlock)
//   D. Feedback status control (FeedbackDetail)
//   E. Staffing add/remove (role=operator, staffing tab)
//
// These three tests are NOT vacuous: each control-presence assertion fails if
// the corresponding {canMutate && …} guard is removed.
//
// Shared fixtures — each row receives a fresh copy via makeCmFalseReports().

/** Build open/resolved/failed report fixtures for canMutate-false tests. */
function makeCmFalseReports() {
  const openReport = {
    id: "00000000-0000-0000-0000-000000000001",
    communityId: "comm-1",
    communityHost: "relay.example.com",
    reportEventId: "ev001",
    reporterPubkey: "rp001",
    targetKind: "event",
    target: "tgt001",
    reportType: "spam",
    status: "open",
    activeAction: null,
    createdAt: "2024-01-01T00:00:00Z",
  };
  const openDetail = {
    ...openReport,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };
  const resolvedReport = {
    ...openReport,
    id: "00000000-0000-0000-0000-000000000002",
    status: "resolved",
  };
  const resolvedDetail = {
    ...resolvedReport,
    channelId: null,
    note: null,
    resolvedBy: "someone",
    resolvedAt: "2024-01-02T00:00:00Z",
    actionId: null,
    message: null,
  };
  const failedAction = {
    id: "act003",
    requestId: "req003",
    actorPubkey: "ac".repeat(32),
    actorRole: "operator",
    action: "ban",
    status: "failed",
    reason: null,
    expiresAt: null,
    errorMessage: "relay error",
    createdAt: "2024-01-01T00:00:00Z",
    updatedAt: "2024-01-01T01:00:00Z",
  };
  const failedReport = {
    ...openReport,
    id: "00000000-0000-0000-0000-000000000003",
    status: "open",
    activeAction: failedAction,
  };
  const failedDetail = {
    ...failedReport,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: "act003",
    message: null,
  };
  return {
    openReport,
    openDetail,
    resolvedReport,
    resolvedDetail,
    failedReport,
    failedDetail,
  };
}

const CM_FALSE_REPORTS_ROWS = [
  {
    name: "resolve",
    description: "resolve-report-form absent in disabled mode",
    // Mutation: remove {canMutate && …} guard on ResolveReportForm → form renders → RED.
    getFixtures: () => {
      const { openReport, openDetail } = makeCmFalseReports();
      return { listItem: openReport, detail: openDetail };
    },
    testId: "resolve-report-form",
    label: "resolve-report-form must be absent when canMutate=false",
  },
  {
    name: "reopen",
    description: "reopen-report-form absent in disabled mode",
    // Mutation: remove {canMutate && …} guard on ReopenReportForm → form renders → RED.
    getFixtures: () => {
      const { resolvedReport, resolvedDetail } = makeCmFalseReports();
      return { listItem: resolvedReport, detail: resolvedDetail };
    },
    testId: "reopen-report-form",
    label: "reopen-report-form must be absent when canMutate=false",
  },
  {
    name: "cancel",
    description: "enforcement-cancel-btn absent in disabled mode",
    // Mutation: remove {canMutate && …} guard on enforcement cancel → button renders → RED.
    getFixtures: () => {
      const { failedReport, failedDetail } = makeCmFalseReports();
      return { listItem: failedReport, detail: failedDetail };
    },
    testId: "enforcement-cancel-btn",
    label: "enforcement-cancel-btn must be absent when canMutate=false",
  },
];

for (const row of CM_FALSE_REPORTS_ROWS) {
  test(`canMutate-false-${row.name}: ${row.description}`, async () => {
    const { listItem, detail } = row.getFixtures();
    setIpcHandler("admin_list_reports", () => Promise.resolve([listItem]));
    setIpcHandler("admin_get_report", () => Promise.resolve(detail));
    setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
    const { container, doRender, unmount } = mountPanel({
      origin: CM_ORIGIN,
      pubkey: CM_PUBKEY,
      canMutate: false,
    });
    try {
      await doRender();
      await settle(30);
      await openFirstReportDetail(container);
      await settle(20);
      assert.equal(
        container.querySelector(`[data-testid='${row.testId}']`),
        null,
        row.label,
      );
    } finally {
      await unmount();
    }
  });
}

// ── P2 round-6 #3: reason audience disclosure ────────────────────────────
//
// Table-driven: each action button selects a disclosure copy. Assertions verify
// both positive presence and negative exclusion of sibling audiences.
// delete has channelId set (Kick/Delete only available for event-in-channel);
// ban uses targetKind "event" with no channel; dismiss uses a pubkey-target.

const REASON_AUDIENCE_ROWS = [
  {
    name: "delete",
    action: "delete",
    targetKind: "event",
    channelId: "00000000-0000-0000-0000-000000000001",
    pubkey: "d1".repeat(32),
    id: "00000000-0000-0000-0000-000000000d01",
    // Mutation: static or affected-user-only copy → room mention absent → RED.
    check: (copy) => {
      assert.ok(
        copy.includes("affected user"),
        `delete audience must mention affected user; got: "${copy}"`,
      );
      assert.ok(
        copy.toLowerCase().includes("publicly in the room"),
        `delete audience must disclose public room; got: "${copy}"`,
      );
    },
  },
  {
    name: "ban",
    action: "ban",
    targetKind: "event",
    channelId: null,
    pubkey: "d2".repeat(32),
    id: "00000000-0000-0000-0000-000000000d02",
    // Mutation: delete-family copy (includes room) for ban → "publicly in the room" present → RED.
    check: (copy) => {
      assert.ok(
        copy.includes("affected user"),
        `ban audience must mention affected user; got: "${copy}"`,
      );
      assert.ok(
        !copy.toLowerCase().includes("publicly in the room"),
        `ban must NOT mention room; got: "${copy}"`,
      );
      assert.ok(
        !copy.toLowerCase().includes("reporter"),
        `ban must NOT mention reporter; got: "${copy}"`,
      );
    },
  },
  {
    name: "dismiss",
    action: "dismiss",
    targetKind: "pubkey",
    channelId: null,
    pubkey: "d3".repeat(32),
    id: "00000000-0000-0000-0000-000000000d03",
    // Mutation: affected-user copy for dismiss → no "reporter" → RED.
    check: (copy) => {
      assert.ok(
        copy.toLowerCase().includes("reporter"),
        `dismiss audience must mention reporter; got: "${copy}"`,
      );
      assert.ok(
        !copy.includes("affected user"),
        `dismiss must NOT mention affected user; got: "${copy}"`,
      );
      assert.ok(
        !copy.toLowerCase().includes("publicly in the room"),
        `dismiss must NOT mention room; got: "${copy}"`,
      );
    },
  },
];

for (const row of REASON_AUDIENCE_ROWS) {
  test(`reason-audience-${row.name}: ${row.name} action shows correct audience disclosure`, async () => {
    const origin = "https://admin.example.com";
    const openItem = {
      id: row.id,
      communityId: "comm-1",
      communityHost: "alpha.example.com",
      reportEventId: "aa",
      reporterPubkey: "bb",
      targetKind: row.targetKind,
      target: "cc",
      reportType: "spam",
      status: "open",
      createdAt: "2024-07-01T00:00:00Z",
    };
    const openDetail = {
      ...openItem,
      channelId: row.channelId,
      note: null,
      resolvedBy: null,
      resolvedAt: null,
      actionId: null,
      message: null,
    };

    setIpcHandler("admin_list_reports", () => Promise.resolve([openItem]));
    setIpcHandler("admin_get_report", () => Promise.resolve(openDetail));
    setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

    const { container, doRender, unmount } = mountPanel({
      origin,
      pubkey: row.pubkey,
    });
    try {
      await doRender();
      await settle(30);
      await openFirstReportDetail(container);
      await settle(20);

      const btn = container.querySelector(
        `[data-testid='action-btn-${row.action}']`,
      );
      assert.ok(btn, `${row.action} action button must be present`);

      await act(async () => {
        fireEvent.click(btn);
        await new Promise((r) => setTimeout(r, 10));
      });

      const audienceEl = container.querySelector(
        "[data-testid='resolve-reason-audience']",
      );
      assert.ok(
        audienceEl !== null,
        `reason audience element must appear after selecting ${row.action}`,
      );
      row.check(audienceEl.textContent ?? "");
    } finally {
      await unmount();
    }
  });
}

// ── P2 round-6 #4: frozen payload, locked controls, authoritative toast ───

test("resolve-frozen-payload-whole: ambiguous failure locks controls and retry sends exact frozen payload", async () => {
  // Verifies Wes finding #4: after an ambiguous failure the action/reason/
  // duration controls are locked, and the retry sends the exact same payload
  // (same requestId, action, reason) without allowing edits.
  //
  // Mutation evidence:
  //   - Not freezing the whole payload (only requestId) → reason can change → RED
  //   - Not disabling controls on ambiguity → locked-controls assertion fails → RED

  const origin = "https://admin.example.com";
  const pubkey = "e1".repeat(32);

  makeOpenReportFixtures("00000000-0000-0000-0000-000000000e01");

  const capturedBodies = [];
  setIpcHandler("admin_resolve_report", (args) => {
    capturedBodies.push({ ...args?.body });
    // Transport failure — no relay answer.
    return mutationReject("network timeout", null);
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    // Select ban and enter a reason.
    const banBtn = container.querySelector("[data-testid='action-btn-ban']");
    assert.ok(banBtn, "ban action button must be present");
    await act(async () => {
      fireEvent.click(banBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const reasonInput = container.querySelector(
      "[data-testid='resolve-reason-input']",
    );
    assert.ok(reasonInput, "reason input must be present");
    await act(async () => {
      fireEvent.change(reasonInput, { target: { value: "original reason" } });
      await new Promise((r) => setTimeout(r, 10));
    });

    const submit = container.querySelector(
      "[data-testid='resolve-submit-btn']",
    );
    assert.ok(submit, "resolve submit button must appear after selecting ban");

    // First attempt — ambiguous failure.
    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(capturedBodies.length, 1, "first attempt must have been made");
    assert.equal(
      capturedBodies[0].action,
      "ban",
      "first attempt must send ban",
    );
    assert.equal(
      capturedBodies[0].reason,
      "original reason",
      "first attempt must send original reason",
    );

    // After ambiguous failure: action/reason controls must be locked.
    const actionBtnsAfter = container.querySelectorAll(
      "[data-testid^='action-btn-']",
    );
    for (const btn of actionBtnsAfter) {
      assert.ok(
        btn.disabled === true,
        `action button ${btn.getAttribute("data-testid")} must be disabled after ambiguous failure; disabled=${btn.disabled}`,
      );
    }
    const reasonInputAfter = container.querySelector(
      "[data-testid='resolve-reason-input']",
    );
    assert.ok(
      reasonInputAfter?.disabled === true,
      "reason input must be disabled after ambiguous failure",
    );

    // Second attempt — frozen payload must be identical.
    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(
      capturedBodies.length,
      2,
      "second attempt must have been made",
    );
    assert.equal(
      capturedBodies[0].requestId,
      capturedBodies[1].requestId,
      `requestId must be identical on retry; got: ${JSON.stringify(capturedBodies.map((b) => b.requestId))}`,
    );
    assert.equal(
      capturedBodies[0].action,
      capturedBodies[1].action,
      `action must be identical on retry; got: ${JSON.stringify(capturedBodies.map((b) => b.action))}`,
    );
    assert.equal(
      capturedBodies[0].reason,
      capturedBodies[1].reason,
      `reason must be identical on retry; got: ${JSON.stringify(capturedBodies.map((b) => b.reason))}`,
    );
  } finally {
    await unmount();
  }
});

test("resolve-toast-from-response-ban: form/response disagree — toast uses relay's ban, not selected dismiss", async () => {
  // Verifies Wes finding #4: the toast derives from AdminReportResolution, not
  // from the mutable form selectedAction.
  //
  // Form disagrees with relay: operator selects Dismiss, but the relay's
  // idempotent response carries activeAction.action = "ban" (the first command
  // that landed). Authoritative path → toast says "Ban". selectedAction path →
  // toast says "Dismiss". The disagreement makes the mutation bite.

  const origin = "https://admin.example.com";
  const pubkey = "e2".repeat(32);

  makeOpenReportFixtures("00000000-0000-0000-0000-000000000e02");

  // Relay returns ban regardless of what the form sent — idempotent first-ban.
  setIpcHandler("admin_resolve_report", () =>
    Promise.resolve({
      status: "resolved",
      activeAction: {
        id: "00000000-0000-0000-0000-0000000000a1",
        requestId: "00000000-0000-0000-0000-000000000001",
        actorPubkey: "e2".repeat(32),
        actorRole: "operator",
        action: "ban",
        status: "succeeded",
        reason: null,
        expiresAt: null,
        errorMessage: null,
        createdAt: "2024-07-01T00:00:00Z",
        updatedAt: "2024-07-01T00:00:00Z",
      },
    }),
  );

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    // Select Dismiss — deliberately different from what the relay will return.
    const dismissBtn = container.querySelector(
      "[data-testid='action-btn-dismiss']",
    );
    assert.ok(dismissBtn, "dismiss action button must be present");
    await act(async () => {
      fireEvent.click(dismissBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const submit = container.querySelector(
      "[data-testid='resolve-submit-btn']",
    );
    assert.ok(
      submit,
      "resolve submit button must appear after selecting dismiss",
    );

    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    // Relay returned ban; toast must say "Ban", not "Dismiss".
    assert.ok(
      capturedToasts.some((m) => m.toLowerCase().includes("ban")),
      `success toast must say "Ban" (from authoritative response, not selected dismiss); got: ${JSON.stringify(capturedToasts)}`,
    );
    assert.ok(
      !capturedToasts.some(
        (m) =>
          m.toLowerCase().includes("dismiss") &&
          !m.toLowerCase().includes("ban"),
      ),
      `toast must not say "Dismiss" when relay returned ban; got: ${JSON.stringify(capturedToasts)}`,
    );
  } finally {
    await unmount();
  }
});

test("resolve-toast-from-response-escalated: retry path — form has dismiss, relay idempotently returns escalated", async () => {
  // Verifies the null-activeAction path after a retry: the frozen form still
  // has "dismiss" selected from the first ambiguous attempt, but the relay
  // idempotently returns {status:"escalated", activeAction:null}.
  //
  // Authoritative path → toast says "Escalate". selectedAction path → toast
  // says "Dismiss". The disagreement makes the mutation bite on the retry.
  //
  // Mutation evidence: change production toast derivation to actionLabel(selectedAction)
  // → with dismiss selected the toast says "Dismiss" even though the relay
  // returned escalated → this test goes RED.

  const origin = "https://admin.example.com";
  const pubkey = "e3".repeat(32);

  makeOpenReportFixtures("00000000-0000-0000-0000-000000000e03", {
    targetKind: "pubkey",
    target: "ff",
  });

  // First attempt: transport error — ambiguous, locks controls and freezes
  // the dismiss payload.
  let attempt = 0;
  setIpcHandler("admin_resolve_report", () => {
    attempt++;
    if (attempt === 1) {
      return mutationReject("relay unreachable: network timeout", null);
    }
    // Second attempt: relay idempotently returns escalated (dismiss was the
    // frozen request; relay previously handled an escalate command).
    return Promise.resolve({ status: "escalated", activeAction: null });
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    // Select Dismiss — this is what gets frozen.
    const dismissBtn = container.querySelector(
      "[data-testid='action-btn-dismiss']",
    );
    assert.ok(dismissBtn, "dismiss action button must be present");
    await act(async () => {
      fireEvent.click(dismissBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const submit = container.querySelector(
      "[data-testid='resolve-submit-btn']",
    );
    assert.ok(
      submit,
      "resolve submit button must appear after selecting dismiss",
    );

    // First attempt — transport error locks the form.
    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(attempt, 1, "first attempt must have fired");

    // Controls must now be locked (frozen payload held).
    const actionBtnsLocked = container.querySelectorAll(
      "[data-testid^='action-btn-']",
    );
    for (const btn of actionBtnsLocked) {
      assert.ok(
        btn.disabled === true,
        `action button ${btn.getAttribute("data-testid")} must be locked after ambiguous failure`,
      );
    }

    // Retry — relay returns escalated while form still shows dismiss.
    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(attempt, 2, "second attempt must have fired");

    // Toast must say "Escalate" (from relay status), not "Dismiss" (from form).
    assert.ok(
      capturedToasts.some((m) => m.toLowerCase().includes("escalate")),
      `success toast must say "Escalate" (from status=escalated, not selected dismiss); got: ${JSON.stringify(capturedToasts)}`,
    );
    assert.ok(
      !capturedToasts.some(
        (m) =>
          m.toLowerCase().includes("dismiss") &&
          !m.toLowerCase().includes("escalate"),
      ),
      `toast must not say "Dismiss" when relay returned escalated; got: ${JSON.stringify(capturedToasts)}`,
    );
  } finally {
    await unmount();
  }
});

test("resolve-definitive-4xx-unlocks-controls: non-409 4xx clears snapshot; corrected resubmit gets fresh ID and body", async () => {
  // Verifies that a definitive pre-commit rejection clears the frozen payload
  // and unlocks action/reason editing. After unlock, a corrected resubmission
  // uses a fresh requestId and the updated action/reason.
  //
  // Mutation evidence: clear frozenRef on EVERY error (not just definitive 4xx)
  // → ambiguity case also unlocks, breaking the frozen-payload invariant.
  // This test verifies the definitive path DOES unlock AND the second call
  // carries different requestId + corrected body.

  const origin = "https://admin.example.com";
  const pubkey = "e4".repeat(32);

  makeOpenReportFixtures("00000000-0000-0000-0000-000000000e04");

  const capturedBodiesE4 = [];
  let callCountE4 = 0;
  setIpcHandler("admin_resolve_report", (args) => {
    callCountE4++;
    capturedBodiesE4.push({ ...args?.body });
    if (callCountE4 === 1) {
      // First call: definitive 400 (relay rejected pre-commit, full body read).
      return mutationReject("bad_request: invalid action", 400);
    }
    // Second call: success after correction.
    return Promise.resolve({
      status: "resolved",
      activeAction: {
        id: "00000000-0000-0000-0000-0000000000b1",
        requestId: capturedBodiesE4[1]?.requestId ?? "",
        actorPubkey: "e4".repeat(32),
        actorRole: "operator",
        action: "ban",
        status: "succeeded",
        reason: "corrected reason",
        expiresAt: null,
        errorMessage: null,
        createdAt: "2024-07-01T00:00:00Z",
        updatedAt: "2024-07-01T00:00:00Z",
      },
    });
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    // First submit: select dismiss, submit → definitive 400.
    const dismissBtn = container.querySelector(
      "[data-testid='action-btn-dismiss']",
    );
    assert.ok(dismissBtn, "dismiss action button must be present");
    await act(async () => {
      fireEvent.click(dismissBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const submit = container.querySelector(
      "[data-testid='resolve-submit-btn']",
    );
    assert.ok(submit, "resolve submit button must appear");

    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(callCountE4, 1, "one attempt must have been made");

    // After a definitive rejection, controls must be unlocked.
    const actionBtnsAfter = container.querySelectorAll(
      "[data-testid^='action-btn-']",
    );
    let anyLocked = false;
    for (const btn of actionBtnsAfter) {
      if (btn.disabled === true) anyLocked = true;
    }
    assert.ok(
      !anyLocked,
      "action buttons must be re-enabled after a definitive pre-commit rejection",
    );

    const reasonInputAfter = container.querySelector(
      "[data-testid='resolve-reason-input']",
    );
    assert.ok(
      reasonInputAfter?.disabled !== true,
      "reason input must be re-enabled after a definitive pre-commit rejection",
    );

    // Corrected resubmit: select ban + enter a new reason.
    const banBtn = container.querySelector("[data-testid='action-btn-ban']");
    assert.ok(banBtn, "ban action button must be present after unlock");
    await act(async () => {
      fireEvent.click(banBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const reasonInput = container.querySelector(
      "[data-testid='resolve-reason-input']",
    );
    assert.ok(reasonInput, "reason input must be present after unlock");
    await act(async () => {
      fireEvent.change(reasonInput, { target: { value: "corrected reason" } });
      await new Promise((r) => setTimeout(r, 10));
    });

    await act(async () => {
      fireEvent.click(submit);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(callCountE4, 2, "two attempts must have been made");

    // Second call must have a FRESH requestId (frozen snapshot was cleared).
    assert.notEqual(
      capturedBodiesE4[0].requestId,
      capturedBodiesE4[1].requestId,
      `corrected resubmit must use a fresh requestId; got: ${JSON.stringify(capturedBodiesE4.map((b) => b.requestId))}`,
    );
    // Second call must carry the corrected action and reason.
    assert.equal(
      capturedBodiesE4[1].action,
      "ban",
      `corrected resubmit must send ban; got: ${capturedBodiesE4[1].action}`,
    );
    assert.equal(
      capturedBodiesE4[1].reason,
      "corrected reason",
      `corrected resubmit must send corrected reason; got: ${capturedBodiesE4[1].reason}`,
    );
  } finally {
    await unmount();
  }
});

// ── Resolve-path whole-payload freeze: 409 / 5xx / truncated ─────────────────
//
// Each ambiguity class (409, 5xx, truncated body) must independently freeze
// the complete Timeout command — requestId, action, reason, and expirationSecs
// — and carry it byte-for-byte on retry.  Using Timeout with a nontrivial
// duration makes expirationSecs a load-bearing field in every case; dropping
// it from the production IPC writer makes all three RED.
//
// Mutation evidence:
//   - Always sending expirationSecs: undefined → deepEqual fails on every case
//   - Resetting frozenRef on ambiguity → requestId differs on second attempt

const FREEZE_DURATION_SECS = 3600;

const RESOLVE_FREEZE_CASES = [
  {
    name: "409-whole-payload",
    desc: "a 409 Conflict is ambiguous: freezes complete Timeout payload, retries byte-for-byte",
    reject: () => mutationReject("admin API error: 409 conflict", 409),
  },
  {
    name: "5xx-whole-payload",
    desc: "a 5xx is ambiguous: freezes complete Timeout payload, retries byte-for-byte",
    reject: () =>
      mutationReject("admin API error: 500 internal server error", 500),
  },
  {
    name: "truncated-body-whole-payload",
    desc: "a truncated/incomplete body (bodyComplete=false) is ambiguous: freezes complete Timeout payload",
    reject: () =>
      mutationReject("admin API error: 400 partial read", 400, false),
  },
];

for (const { name, desc, reject: makeReject } of RESOLVE_FREEZE_CASES) {
  test(`resolve-${name}: ${desc}`, async () => {
    const origin = "https://admin.example.com";
    const pubkey = `e5${name.slice(0, 6).replace(/-/g, "0")}`.padEnd(64, "5");

    makeOpenReportFixtures(
      `00000000-0000-0000-0000-${name.replace(/-/g, "").slice(0, 12).padStart(12, "0")}`,
    );

    const capturedFreezeBodies = [];
    setIpcHandler("admin_resolve_report", (args) => {
      capturedFreezeBodies.push({ ...args?.body });
      return makeReject();
    });

    const { container, doRender, unmount } = mountPanel({ origin, pubkey });
    try {
      await doRender();
      await settle(30);
      await openFirstReportDetail(container);
      await settle(20);

      // Select Timeout so expirationSecs is part of the frozen payload.
      const timeoutBtn = container.querySelector(
        "[data-testid='action-btn-timeout']",
      );
      assert.ok(timeoutBtn, "timeout action button must be present");
      await act(async () => {
        fireEvent.click(timeoutBtn);
        await new Promise((r) => setTimeout(r, 10));
      });

      const durationInput = container.querySelector(
        "[data-testid='timeout-duration-input']",
      );
      assert.ok(durationInput, "timeout duration input must appear");
      await act(async () => {
        fireEvent.change(durationInput, {
          target: { value: String(FREEZE_DURATION_SECS) },
        });
        await new Promise((r) => setTimeout(r, 10));
      });

      const reasonInput = container.querySelector(
        "[data-testid='resolve-reason-input']",
      );
      assert.ok(reasonInput, "reason input must be present");
      await act(async () => {
        fireEvent.change(reasonInput, {
          target: { value: "freeze-test reason" },
        });
        await new Promise((r) => setTimeout(r, 10));
      });

      const submit = container.querySelector(
        "[data-testid='resolve-submit-btn']",
      );
      assert.ok(
        submit,
        "resolve submit button must appear after selecting timeout",
      );

      // First attempt — ambiguous failure freezes the complete payload.
      await act(async () => {
        fireEvent.click(submit);
        await new Promise((r) => setTimeout(r, 20));
      });

      assert.equal(
        capturedFreezeBodies.length,
        1,
        `[${name}] first attempt must have been made`,
      );
      assert.equal(
        capturedFreezeBodies[0].action,
        "timeout",
        `[${name}] first attempt must send timeout`,
      );
      assert.equal(
        capturedFreezeBodies[0].expirationSecs,
        FREEZE_DURATION_SECS,
        `[${name}] first attempt must include expirationSecs=${FREEZE_DURATION_SECS}`,
      );

      // After ambiguous failure: action, reason, and duration controls must be locked.
      const actionBtnsAfter = container.querySelectorAll(
        "[data-testid^='action-btn-']",
      );
      for (const btn of actionBtnsAfter) {
        assert.ok(
          btn.disabled === true,
          `[${name}] action button ${btn.getAttribute("data-testid")} must be disabled after ambiguous failure`,
        );
      }
      const reasonInputAfter = container.querySelector(
        "[data-testid='resolve-reason-input']",
      );
      assert.ok(
        reasonInputAfter?.disabled === true,
        `[${name}] reason input must be disabled after ambiguous failure`,
      );
      const durationInputAfter = container.querySelector(
        "[data-testid='timeout-duration-input']",
      );
      assert.ok(
        durationInputAfter?.disabled === true,
        `[${name}] duration input must be disabled after ambiguous failure`,
      );

      // Second attempt — retry must send the complete frozen payload byte-for-byte.
      await act(async () => {
        fireEvent.click(submit);
        await new Promise((r) => setTimeout(r, 20));
      });

      assert.equal(
        capturedFreezeBodies.length,
        2,
        `[${name}] two attempts must have been made`,
      );
      assert.deepEqual(
        capturedFreezeBodies[1],
        capturedFreezeBodies[0],
        `[${name}] retry must send the complete frozen payload (requestId+action+reason+expirationSecs); got: ${JSON.stringify(capturedFreezeBodies)}`,
      );
    } finally {
      await unmount();
    }
  });
}

// ── Item 4: kick AlreadyGone friendly copy ────────────────────────────────

test("kick-already-gone-friendly-copy: enforcement block shows friendly message for kick target already absent", async () => {
  // The relay stores the raw anyhow error string on a failed kick:
  // "kick target was already absent before this action". The UI must
  // translate this to the operator-facing recommendation.

  const origin = "https://admin.example.com";
  const pubkey = "f9".repeat(32);

  const base = makeReportBase({ id: "00000000-0000-0000-0000-000000000ff9" });
  const kickGoneDetail = {
    ...base,
    status: "processing",
    channelId: "00000000-0000-0000-0000-0000000000cc",
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    activeAction: {
      id: "00000000-0000-0000-0000-0000000000fd",
      requestId: "00000000-0000-0000-0000-0000000000fe",
      actorPubkey:
        "1111111111111111111111111111111111111111111111111111111111111111",
      actorRole: "operator",
      action: "kick",
      status: "failed",
      reason: null,
      expiresAt: null,
      errorMessage: "kick target was already absent before this action",
      createdAt: "2024-06-01T12:00:00Z",
      updatedAt: "2024-06-01T12:00:05Z",
    },
    message: null,
  };

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "processing" }]),
  );
  setIpcHandler("admin_get_report", () => Promise.resolve(kickGoneDetail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    const block = container.querySelector(
      "[data-testid='enforcement-state-block']",
    );
    assert.ok(block, "enforcement-state-block must render for a failed kick");

    const text = block.textContent ?? "";
    assert.ok(
      text.includes("No channel membership to remove"),
      `enforcement block must show friendly kick-absent copy; got: ${text}`,
    );
    assert.ok(
      !text.includes("was already absent"),
      `enforcement block must NOT show raw relay error; got: ${text}`,
    );
  } finally {
    await unmount();
  }
});

// ── Item 7: report list row snippet and detail PubKey rendering ───────────

test("report-list-snippet: list rows show truncated pubkey fallback when profiles absent", async () => {
  // Without a resolved profile the snippet falls back to the first 8 chars of
  // the hex pubkey (truncatePubkey) so rows are still distinguishable.

  const origin = "https://admin.example.com";
  const pubkey = "fa".repeat(32);

  const reporterHex = "1234567890abcdef".repeat(4); // 64 hex
  const targetHex = "fedcba0987654321".repeat(4); // 64 hex
  const base = makeReportBase({
    id: "00000000-0000-0000-0000-000000000ffa",
    reporterPubkey: reporterHex,
    target: targetHex,
    targetKind: "pubkey",
  });

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "open" }]),
  );
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);

    // Activate Reports tab.
    const tabs = Array.from(container.querySelectorAll("button"));
    const reportsTab = tabs.find((b) =>
      (b.getAttribute("data-testid") ?? "").includes("tab-reports"),
    );
    assert.ok(reportsTab, "reports tab button must exist");
    await act(async () => {
      fireEvent.click(reportsTab);
      await new Promise((r) => setTimeout(r, 20));
    });

    const listItems = container.querySelectorAll("li button");
    assert.ok(listItems.length > 0, "at least one report list row must render");
    const rowText = listItems[0].textContent ?? "";
    // The reporter snippet is the first 8 chars of the hex.
    assert.ok(
      rowText.includes(reporterHex.slice(0, 8)),
      `list row must include reporter hex snippet; got: ${rowText}`,
    );
    // The target snippet is the first 8 chars of the target hex.
    assert.ok(
      rowText.includes(targetHex.slice(0, 8)),
      `list row must include target hex snippet; got: ${rowText}`,
    );
  } finally {
    await unmount();
  }
});

test("report-list-snippet: list rows show display names when profiles resolve", async () => {
  // When get_users_batch returns display names for reporter and target
  // pubkeys, the list row snippet must render those names instead of
  // truncated hex.

  const origin = "https://admin-display-names.example.com";
  const pubkey = "fa".repeat(32);

  const reporterHex = "1234567890abcdef".repeat(4); // 64 hex
  const targetHex = "fedcba0987654321".repeat(4); // 64 hex
  const base = makeReportBase({
    id: "00000000-0000-0000-0000-000000000ffb",
    reporterPubkey: reporterHex,
    target: targetHex,
    targetKind: "pubkey",
  });

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([{ ...base, status: "open" }]),
  );
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("get_users_batch", (args) => {
    const profiles = {};
    for (const pk of args?.pubkeys ?? []) {
      if (pk === reporterHex) {
        profiles[pk] = { display_name: "Alice Reporter", avatar_url: null };
      } else if (pk === targetHex) {
        profiles[pk] = { display_name: "Bob Target", avatar_url: null };
      }
    }
    return Promise.resolve({ profiles, missing: [] });
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(50);
    await settle(100);

    // Activate Reports tab.
    const tabs = Array.from(container.querySelectorAll("button"));
    const reportsTab = tabs.find((b) =>
      (b.getAttribute("data-testid") ?? "").includes("tab-reports"),
    );
    assert.ok(reportsTab, "reports tab button must exist");
    await act(async () => {
      fireEvent.click(reportsTab);
      await new Promise((r) => setTimeout(r, 20));
    });
    await settle(100);

    const listItems = container.querySelectorAll("li button");
    assert.ok(listItems.length > 0, "at least one report list row must render");
    const rowText = listItems[0].textContent ?? "";
    assert.ok(
      rowText.includes("Alice Reporter"),
      `list row must show reporter display name "Alice Reporter"; got: ${rowText}`,
    );
    assert.ok(
      rowText.includes("Bob Target"),
      `list row must show target display name "Bob Target"; got: ${rowText}`,
    );
  } finally {
    await unmount();
  }
});

test("report-list-snippet: event reports name the reported author, never look up the event id", async () => {
  const origin = "https://admin-event-author.example.com";
  const pubkey = "fa".repeat(32);

  const reporterHex = "1234567890abcdef".repeat(4);
  const eventIdHex = "fedcba0987654321".repeat(4);
  const authorHex = "a1b2c3d4e5f60718".repeat(4);
  const base = makeReportBase({
    id: "00000000-0000-0000-0000-000000000ffc",
    reporterPubkey: reporterHex,
    target: eventIdHex,
    targetKind: "event",
  });

  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([
      { ...base, status: "open", targetAuthorPubkey: authorHex },
    ]),
  );
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  const profileRequests = [];
  setIpcHandler("get_users_batch", (args) => {
    profileRequests.push(...(args?.pubkeys ?? []));
    const names = {
      [reporterHex]: "Alice Reporter",
      [authorHex]: "Carol Author",
      [eventIdHex]: "Not A Person",
    };
    const profiles = {};
    for (const pk of args?.pubkeys ?? []) {
      if (names[pk])
        profiles[pk] = { display_name: names[pk], avatar_url: null };
    }
    return Promise.resolve({ profiles, missing: [] });
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(50);
    await settle(100);
    const reportsTab = Array.from(container.querySelectorAll("button")).find(
      (b) => (b.getAttribute("data-testid") ?? "").includes("tab-reports"),
    );
    assert.ok(reportsTab, "reports tab button must exist");
    await act(async () => {
      fireEvent.click(reportsTab);
      await new Promise((r) => setTimeout(r, 20));
    });
    await settle(100);

    assert.equal(
      profileRequests.includes(eventIdHex),
      false,
      "an event id must never be sent to the profile lookup",
    );
    const rowText = container.querySelector("li button")?.textContent ?? "";
    assert.ok(
      rowText.includes("Alice Reporter"),
      `reporter name; got: ${rowText}`,
    );
    assert.ok(
      rowText.includes("Carol Author"),
      `reported author name; got: ${rowText}`,
    );
    assert.ok(
      !rowText.includes("Not A Person"),
      `event id resolved as a person; got: ${rowText}`,
    );
  } finally {
    await unmount();
  }
});

test("report-detail-event-target: event targets show and copy the raw event ID, not an npub", async () => {
  // Mutation evidence: routing event targets back through <PubKey> renders
  // npub(E) and drops the copy control, failing both assertions.
  const origin = "https://admin.example.com";
  const pubkey = "d4".repeat(32);
  const eventId = "e1".repeat(32);
  const reportId = "00000000-0000-0000-0000-0000000000e1";
  const item = {
    id: reportId,
    communityId: "comm-1",
    communityHost: "alpha.example.com",
    reportEventId: "f2".repeat(32),
    reporterPubkey: "bb",
    targetKind: "event",
    target: eventId,
    reportType: "spam",
    status: "open",
    createdAt: "2024-06-01T12:00:00Z",
  };
  const detail = {
    ...item,
    channelId: null,
    note: null,
    resolvedBy: null,
    resolvedAt: null,
    actionId: null,
    message: null,
  };
  setIpcHandler("admin_list_reports", () => Promise.resolve([item]));
  setIpcHandler("admin_get_report", () => Promise.resolve(detail));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  const copied = [];
  setIpcHandler("copy_text_to_clipboard", (args) => {
    copied.push(args?.text);
    return Promise.resolve();
  });

  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    await openFirstReportDetail(container);
    await settle(20);

    assert.equal(
      container.querySelector("[data-testid='report-target-pubkey']"),
      null,
    );
    const shown = container.querySelector(
      "[data-testid='report-target-event-id']",
    );
    assert.ok(shown, "event target must render as a raw event ID");
    assert.equal(shown.textContent, eventId);
    assert.ok(
      !container.textContent.includes("npub1"),
      "no npub for an event target",
    );

    const copy = container.querySelector(
      "[data-testid='report-target-event-copy']",
    );
    assert.ok(copy, "event target must offer a copy control");
    await act(async () => {
      copy.click();
    });
    await settle(10);
    assert.deepEqual(copied, [eventId]);
  } finally {
    await unmount();
  }
});
