/**
 * Staffing tab behavior tests for AdminConsolePanel. Covers operator
 * add/remove/role-change, display-name integration, self-removal callback,
 * dialog confirmation, canMutate gates, and tab reset on role downgrade.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  React,
  act,
  fireEvent,
  createRoot,
  QueryClientProvider,
  CommunitiesProvider,
  AdminConsolePanel,
  setIpcHandler,
  resetTestState,
  mutationReject,
  makeQueryClient,
  mountPanel,
  mountStaffingPanel,
  settle,
  CM_ORIGIN,
  CM_PUBKEY,
  CM_OP_PUBKEY,
  TEST_RELAY_WS_URL,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";

afterEach(resetTestState);

test("canMutate-false-staffing: staffing add/remove absent in disabled mode", async () => {
  // Mutation: remove {canMutate && …} guards on staffing add/remove → buttons render → RED.
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve([
      { pubkey: CM_OP_PUBKEY, effectiveRole: "moderator", sources: ["db"] },
    ]),
  );
  const { container, doRender, unmount } = mountPanel({
    origin: CM_ORIGIN,
    pubkey: CM_PUBKEY,
    canMutate: false,
    role: "operator",
    initialTab: "staffing",
  });
  try {
    await doRender();
    await settle(30);
    assert.equal(
      container.querySelector("[data-testid='staffing-add-btn']"),
      null,
      "staffing-add-btn must be absent when canMutate=false",
    );
    assert.equal(
      container.querySelector(
        `[data-testid='staffing-remove-btn-${CM_OP_PUBKEY}']`,
      ),
      null,
      "staffing-remove-btn must be absent when canMutate=false",
    );
  } finally {
    await unmount();
  }
});

// ── P1: Staffing remove confirmation dialog ───────────────────────────────────
//
// The trash button must open a confirmation dialog; the delete IPC must not fire
// until the user clicks Confirm. Self-removal shows a distinct warning.
//
// Mutation evidence:
//   - Bypass the dialog (call deleteAdminOperator directly from the button) →
//     the cancel test goes RED (deleteAdminOperator called on trash click).
//   - Remove the AlertDialog open condition → confirm test goes RED (dialog
//     never opens, Confirm button absent).

test("staffing-remove-cancel: trash click opens dialog; cancel does not invoke deleteAdminOperator", async () => {
  const origin = "https://admin-staffing.example.com";
  const pubkey = "aa".repeat(32);
  const opPubkey = "bb".repeat(32);

  const deleteCalls = [];
  setIpcHandler("admin_delete_operator", (args) => {
    deleteCalls.push(args?.pubkey ?? "?");
    return Promise.resolve();
  });

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey, [
    { pubkey: opPubkey, effectiveRole: "moderator", sources: ["db"] },
  ]);
  await doRender();
  await settle(30);

  try {
    // Trash click → dialog opens (no delete yet)
    const removeBtn = container.querySelector(
      `[data-testid='staffing-remove-btn-${opPubkey}']`,
    );
    assert.ok(
      removeBtn !== null,
      "remove button must be present before dialog",
    );
    await act(async () => {
      fireEvent.click(removeBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    // Dialog should be open — content renders in document.body portal
    const dialog = document.body.querySelector(
      "[data-testid='staffing-remove-dialog']",
    );
    assert.ok(
      dialog !== null,
      "confirmation dialog must open after trash click",
    );
    assert.equal(
      deleteCalls.length,
      0,
      "deleteAdminOperator must not fire before confirmation",
    );

    // Click Cancel
    const cancelBtn = document.body.querySelector(
      "[data-testid='staffing-remove-cancel']",
    );
    assert.ok(cancelBtn !== null, "cancel button must be present in dialog");
    await act(async () => {
      fireEvent.click(cancelBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    // Dialog closed, row still present, delete still not called
    const dialogAfter = document.body.querySelector(
      "[data-testid='staffing-remove-dialog']",
    );
    assert.equal(dialogAfter, null, "dialog must close after cancel");
    assert.equal(
      deleteCalls.length,
      0,
      "deleteAdminOperator must not be invoked after cancel",
    );
    const rowAfter = container.querySelector(
      `[data-testid='staffing-row-${opPubkey}']`,
    );
    assert.ok(
      rowAfter !== null,
      "operator row must still be present after cancel",
    );
  } finally {
    await unmount();
  }
});

test("staffing-remove-confirm: confirming dialog invokes deleteAdminOperator exactly once with the right pubkey", async () => {
  const origin = "https://admin-staffing.example.com";
  const pubkey = "cc".repeat(32);
  const opPubkey = "dd".repeat(32);

  const deleteCalls = [];
  setIpcHandler("admin_delete_operator", (args) => {
    deleteCalls.push(args?.pubkey ?? "?");
    return Promise.resolve();
  });

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey, [
    { pubkey: opPubkey, effectiveRole: "moderator", sources: ["db"] },
  ]);
  await doRender();
  await settle(30);

  try {
    // Open dialog
    const removeBtn = container.querySelector(
      `[data-testid='staffing-remove-btn-${opPubkey}']`,
    );
    assert.ok(removeBtn !== null, "remove button must be present");
    await act(async () => {
      fireEvent.click(removeBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const dialog = document.body.querySelector(
      "[data-testid='staffing-remove-dialog']",
    );
    assert.ok(dialog !== null, "confirmation dialog must be open");

    // Click Confirm
    const confirmBtn = document.body.querySelector(
      "[data-testid='staffing-remove-confirm']",
    );
    assert.ok(confirmBtn !== null, "confirm button must be present in dialog");
    await act(async () => {
      fireEvent.click(confirmBtn);
      await new Promise((r) => setTimeout(r, 30));
    });

    // deleteAdminOperator must have been called exactly once with the right pubkey
    assert.equal(
      deleteCalls.length,
      1,
      `deleteAdminOperator must be invoked exactly once; calls: ${JSON.stringify(deleteCalls)}`,
    );
    assert.equal(
      deleteCalls[0],
      opPubkey,
      `deleteAdminOperator must receive the target pubkey; got: ${deleteCalls[0]}`,
    );
  } finally {
    await unmount();
  }
});

test("staffing-remove-self-warning: self-removal dialog shows the distinct self-removal warning", async () => {
  const origin = "https://admin-staffing.example.com";
  // acting pubkey == op pubkey → self-removal
  const pubkey = "ee".repeat(32);

  const deleteCalls = [];
  setIpcHandler("admin_delete_operator", (args) => {
    deleteCalls.push(args?.pubkey ?? "?");
    return Promise.resolve();
  });

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey, [
    { pubkey: pubkey, effectiveRole: "operator", sources: ["db"] },
  ]);
  await doRender();
  await settle(30);

  try {
    // Open dialog for the acting user's own row
    const removeBtn = container.querySelector(
      `[data-testid='staffing-remove-btn-${pubkey}']`,
    );
    assert.ok(removeBtn !== null, "own remove button must be present");
    await act(async () => {
      fireEvent.click(removeBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const warning = document.body.querySelector(
      "[data-testid='staffing-remove-self-warning']",
    );
    assert.ok(
      warning !== null,
      "self-removal warning must appear when removing own operator access",
    );
  } finally {
    await unmount();
  }
});

// ── P2: activeTab resets when role transitions out of staffing ────────────────
//
// If a mounted panel transitions from operator → moderator/unknown while
// Staffing is selected, the panel must reset to reports rather than leaving
// an empty/invisible state.
//
// Mutation evidence: removing the reset useEffect → this test goes RED
// (no tab content renders after the role downgrade).

test("staffing-tab-reset-on-role-downgrade: panel shows reports content after operator→moderator transition", async () => {
  const origin = "https://admin-rw.example.com";
  const pubkey = "ff".repeat(32);

  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  // Mount with operator role + staffing tab active
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const qc = makeQueryClient(pubkey);

  const renderWith = async (role) => {
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(AdminConsolePanel, {
              canMutate: true,
              origin,
              pubkey,
              role,
              initialTab: "staffing",
            }),
          ),
        ),
      );
    });
  };
  const unmount = async () => {
    await act(async () => {
      root.unmount();
    });
    document.body.removeChild(container);
  };

  try {
    await renderWith("operator");
    await settle(30);

    // Staffing tab content is visible
    const staffingContent = container.querySelector(
      "[data-testid='staffing-tab']",
    );
    assert.ok(
      staffingContent !== null,
      "staffing tab content must be visible when role=operator",
    );

    // Transition to moderator — staffing tab is now unauthorized
    await renderWith("moderator");
    await settle(20);

    // Staffing content must be gone; reports content must be present
    const staffingAfter = container.querySelector(
      "[data-testid='staffing-tab']",
    );
    assert.equal(
      staffingAfter,
      null,
      "staffing tab content must be absent after role downgrade to moderator",
    );

    // The reset effect must have switched activeTab → reports, so the reports
    // tab wrapper must be in the DOM. Without the reset, activeTab stays on
    // staffing and neither staffing (gated by isOperator) nor reports renders.
    const reportsTabContent = container.querySelector(
      "[data-testid='reports-tab']",
    );
    assert.ok(
      reportsTabContent !== null,
      "reports-tab content must render after reset (without reset, panel is empty)",
    );

    // The reports tab button must exist and not the staffing tab button
    const reportsTabBtn = container.querySelector(
      "[data-testid='admin-tab-reports']",
    );
    assert.ok(
      reportsTabBtn !== null,
      "reports tab button must be visible after reset",
    );
    const staffingTabBtn = container.querySelector(
      "[data-testid='admin-tab-staffing']",
    );
    assert.equal(
      staffingTabBtn,
      null,
      "staffing tab button must be absent after role downgrade to moderator",
    );
  } finally {
    await unmount();
  }
});

// ── P1: Staffing add is create-only — duplicate guard ────────────────────────
//
// Submitting an operator pubkey already present in the loaded roster must
// produce zero PUTs and surface a specific inline error naming the effective
// role. Submitting a new pubkey must produce exactly one PUT with the complete
// body. The Add button must be disabled until the list loads successfully.
//
// Mutation evidence:
//   - Removing the duplicate-guard `if (existing)` block → zero-PUT assertion
//     fails when an existing key is submitted (a PUT fires instead).

test("staffing-add-duplicate-guard: submitting an existing key produces zero PUTs; submitting a new key produces one complete PUT", async () => {
  const origin = "https://admin-staffing.example.com";
  const pubkey = "11".repeat(32);
  const existingPubkey = "22".repeat(32);
  const newPubkey = "33".repeat(32);

  const putCalls = [];
  const roster = [
    {
      pubkey: existingPubkey,
      effectiveRole: "operator",
      sources: ["db"],
    },
    {
      pubkey: "44".repeat(32),
      effectiveRole: "moderator",
      sources: ["db"],
    },
  ];
  setIpcHandler("admin_put_operator", (args) => {
    putCalls.push({ pubkey: args?.pubkey, role: args?.body?.role });
    const newEntry = {
      pubkey: args?.pubkey,
      effectiveRole: args?.body?.role,
      sources: ["db"],
    };
    roster.push(newEntry);
    return Promise.resolve(newEntry);
  });

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    roster,
  );
  await doRender();
  await settle(30);

  try {
    // ── Case 1: submit an existing pubkey with the default role (moderator) ──
    const pubkeyInput = container.querySelector(
      "[data-testid='staffing-add-pubkey-input']",
    );
    assert.ok(pubkeyInput, "pubkey input must be present");

    await act(async () => {
      fireEvent.change(pubkeyInput, { target: { value: existingPubkey } });
      await new Promise((r) => setTimeout(r, 10));
    });

    const addBtn = container.querySelector("[data-testid='staffing-add-btn']");
    assert.ok(addBtn, "Add button must be present");

    await act(async () => {
      fireEvent.click(addBtn);
      await new Promise((r) => setTimeout(r, 20));
    });

    assert.equal(
      putCalls.length,
      0,
      "admin_put_operator must NOT be called for an existing pubkey",
    );

    // An inline error naming the existing effective role must be visible.
    const errEls = Array.from(
      container.querySelectorAll(
        "[data-testid='staffing-tab'] .text-destructive",
      ),
    );
    assert.ok(
      errEls.some((el) => el.textContent.includes("operator")),
      `inline error must name the existing effective role "operator"; found: ${errEls.map((e) => e.textContent).join(", ")}`,
    );

    // The existing row must still be present with its original role after the
    // rejected duplicate submit — the roster must be unmodified.
    await settle(10);
    const existingRow = container.querySelector(
      `[data-testid='staffing-row-${existingPubkey}']`,
    );
    assert.ok(
      existingRow !== null,
      "existing operator row must still render after duplicate-submit rejection",
    );
    assert.ok(
      existingRow.textContent.includes("operator"),
      `existing row must still show the "operator" role after rejection; got: ${existingRow.textContent}`,
    );

    // ── Case 2: clear the input and submit a genuinely new pubkey ──
    await act(async () => {
      fireEvent.change(pubkeyInput, { target: { value: newPubkey } });
      await new Promise((r) => setTimeout(r, 10));
    });

    await act(async () => {
      fireEvent.click(addBtn);
      await new Promise((r) => setTimeout(r, 30));
    });

    assert.equal(
      putCalls.length,
      1,
      `admin_put_operator must be called exactly once for a new pubkey; got ${putCalls.length}`,
    );
    assert.equal(
      putCalls[0].pubkey,
      newPubkey,
      `PUT must carry the new pubkey; got: ${putCalls[0].pubkey}`,
    );
    assert.equal(
      putCalls[0].role,
      "moderator",
      `PUT must carry the selected role; got: ${putCalls[0].role}`,
    );

    // The row for the new pubkey must appear (list refreshed).
    await settle(30);
    const newRow = container.querySelector(
      `[data-testid='staffing-row-${newPubkey}']`,
    );
    assert.ok(
      newRow !== null,
      "new operator row must appear after successful PUT",
    );
  } finally {
    await unmount();
  }
});

// ── P2: Staffing display-name + npub presentation ─────────────────────────────
//
// Behavioral coverage for the useUsersBatchQuery integration.
//
// Mutation evidence:
//   - Suppress the get_users_batch IPC response → display name test goes RED
//     (raw pubkey renders instead of display name).
//   - Remove HoverStaffingIdentity → npub data-testid absent → npub test RED.
//   - Remove putAdminOperator call from handleRoleChange → PUT test goes RED.
//   - Swap 409 check for generic message → rejection copy test goes RED.

test("staffing-display-name: resolved profile name renders in place of raw pubkey", async () => {
  // Verifies that get_users_batch is called and the returned displayName renders
  // in the staffing row — not the fallback truncated pubkey.
  const origin = "https://admin-staffing-name.example.com";
  const pubkey = "a1".repeat(32);
  const opPubkey = "b2".repeat(32);

  setIpcHandler("get_users_batch", (args) => {
    const profiles = {};
    for (const pk of args?.pubkeys ?? []) {
      if (pk === opPubkey) {
        // Raw IPC format uses snake_case (getRawUsersBatchResponse shape).
        profiles[pk] = { display_name: "Alice Operator", avatar_url: null };
      }
    }
    return Promise.resolve({ profiles, missing: [] });
  });

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey, [
    { pubkey: opPubkey, effectiveRole: "moderator", sources: ["db"] },
  ]);
  await doRender();
  // admin_list_operators resolves first, populating listedPubkeys, which enables
  // useUsersBatchQuery. A second settle cycle lets React Query fire get_users_batch
  // and commit the result before the assertion.
  await settle(50);
  await settle(100);

  try {
    const nameEl = container.querySelector(
      `[data-testid='staffing-name-${opPubkey}']`,
    );
    assert.ok(
      nameEl !== null,
      "staffing-name element must be present for listed operator",
    );
    assert.ok(
      nameEl.textContent.includes("Alice Operator"),
      `staffing row must render resolved display name "Alice Operator"; got: "${nameEl.textContent}"`,
    );
    // The npub span must also be present alongside the display name.
    // Folded from staffing-npub-hover: the DOM node must exist and start with "npub1".
    const npubEl = container.querySelector(
      `[data-testid='staffing-npub-${opPubkey}']`,
    );
    assert.ok(
      npubEl !== null,
      "staffing-npub element must be present for listed operator",
    );
    assert.ok(
      npubEl.textContent.startsWith("npub1") ||
        npubEl.textContent.includes("npub"),
      `staffing-npub must contain encoded npub prefix; got: "${npubEl.textContent}"`,
    );
  } finally {
    await unmount();
  }
});

test("staffing-role-change-success: role selector change calls putAdminOperator and refreshes the list", async () => {
  // Verifies that selecting a different role triggers one PUT with the new role
  // and the row reflects the update after the list refresh.
  const origin = "https://admin-staffing-role.example.com";
  const pubkey = "e5".repeat(32);
  const opPubkey = "f6".repeat(32);

  const putCalls = [];
  let currentRole = "moderator";
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_operators", () =>
    Promise.resolve([
      { pubkey: opPubkey, effectiveRole: currentRole, sources: ["db"] },
    ]),
  );
  setIpcHandler("admin_put_operator", (args) => {
    putCalls.push({ pubkey: args?.pubkey, role: args?.body?.role });
    currentRole = args?.body?.role;
    return Promise.resolve({
      pubkey: opPubkey,
      effectiveRole: currentRole,
      sources: ["db"],
    });
  });

  const { container, doRender, unmount } = mountPanel({
    origin,
    pubkey,
    canMutate: true,
    role: "operator",
    initialTab: "staffing",
  });
  await doRender();
  await settle(30);

  try {
    const roleSelect = container.querySelector(
      `[data-testid='staffing-role-select-${opPubkey}']`,
    );
    assert.ok(
      roleSelect !== null,
      "role selector must be present for DB-backed operator in canMutate mode",
    );

    // Change to operator
    await act(async () => {
      fireEvent.change(roleSelect, { target: { value: "operator" } });
      await new Promise((r) => setTimeout(r, 30));
    });

    assert.equal(
      putCalls.length,
      1,
      `admin_put_operator must be called exactly once on role change; got ${putCalls.length}`,
    );
    assert.equal(
      putCalls[0].pubkey,
      opPubkey,
      `PUT must carry the operator pubkey; got: ${putCalls[0].pubkey}`,
    );
    assert.equal(
      putCalls[0].role,
      "operator",
      `PUT must carry the new role "operator"; got: ${putCalls[0].role}`,
    );

    // After list refresh the role selector must reflect the updated role
    await settle(30);
    const roleSelectAfter = container.querySelector(
      `[data-testid='staffing-role-select-${opPubkey}']`,
    );
    assert.ok(
      roleSelectAfter !== null,
      "role selector must still be present after refresh",
    );
    assert.equal(
      roleSelectAfter.value,
      "operator",
      `role selector must show updated role "operator" after refresh; got: ${roleSelectAfter.value}`,
    );
  } finally {
    await unmount();
  }
});

test("staffing-role-change-409: putAdminOperator error cases surface the relay message", async () => {
  // Verifies that role-change errors surface the relay's parsed error message
  // directly, not a hardcoded "config-backed" copy.
  //
  // Mutation evidence:
  //   - Restore the old adminMutationRelayStatus === 409 branch →
  //     last-operator row shows "config-backed" instead of the relay message → RED.
  //   - Remove the adminErrorMessage(e) call → raw JSON renders → RED.
  const origin = "https://admin-staffing-role-reject.example.com";
  const pubkey = "07".repeat(32);
  const opPubkey = "18".repeat(32);

  const ROLE_CHANGE_ERROR_ROWS = [
    {
      name: "config-backed 409",
      message:
        'admin API error: {"error":{"code":"conflict","message":"pubkey is backed by config (RELAY_OPERATOR_PUBKEYS or owner fallback) — immutable through the API"}}',
      status: 409,
      contains: "immutable through the API",
    },
    {
      name: "last-operator 409",
      message:
        'admin API error: {"error":{"code":"conflict","message":"operation would remove the last relay operator — add a replacement operator first"}}',
      status: 409,
      contains: "add a replacement operator first",
    },
  ];

  let putResult = () => mutationReject(ROLE_CHANGE_ERROR_ROWS[0].message, 409);
  setIpcHandler("admin_put_operator", () => putResult());

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey, [
    { pubkey: opPubkey, effectiveRole: "moderator", sources: ["db"] },
  ]);
  await doRender();
  await settle(30);

  try {
    const roleSelect = container.querySelector(
      `[data-testid='staffing-role-select-${opPubkey}']`,
    );
    assert.ok(roleSelect !== null, "role selector must be present");

    for (const row of ROLE_CHANGE_ERROR_ROWS) {
      putResult = () => mutationReject(row.message, row.status);
      // Re-select moderator first so the change is non-trivial.
      await act(async () => {
        fireEvent.change(roleSelect, { target: { value: "moderator" } });
        await new Promise((r) => setTimeout(r, 10));
      });
      const roleSelectCurrent = container.querySelector(
        `[data-testid='staffing-role-select-${opPubkey}']`,
      );
      await act(async () => {
        fireEvent.change(roleSelectCurrent, { target: { value: "operator" } });
        await new Promise((r) => setTimeout(r, 30));
      });

      const errEls = Array.from(
        container.querySelectorAll(
          "[data-testid='staffing-tab'] [class*='destructive']",
        ),
      );
      assert.ok(
        errEls.length > 0,
        `an error element must appear after rejected role change (${row.name})`,
      );
      assert.ok(
        errEls.some((el) => el.textContent.includes(row.contains)),
        `${row.name} must surface relay message containing "${row.contains}"; got: ${errEls.map((e) => e.textContent).join(", ")}`,
      );
      assert.ok(
        !errEls.some((el) => el.textContent.includes("admin API error")),
        `${row.name}: raw envelope prefix must not render`,
      );
    }
  } finally {
    await unmount();
  }
});

test("staffing-add-409: putAdminOperator error cases surface the relay message", async () => {
  // handleAdd surfaces adminErrorMessage(e) for ALL errors — a 409 shows the
  // relay's parsed message (config-backed OR last-operator conflict), not a
  // hardcoded copy.
  //
  // Mutation evidence:
  //   - Restore the old adminMutationRelayStatus === 409 hardcode →
  //     last-operator row shows "config-backed" not the relay message → RED.
  //   - Remove adminErrorMessage(e) → raw JSON envelope renders → RED.
  const origin = "https://admin-staffing-add-reject.example.com";
  const pubkey = "07".repeat(32);

  const ADD_ERROR_ROWS = [
    {
      name: "config-backed 409",
      pubkeyInput: "19".repeat(32),
      message:
        'admin API error: {"error":{"code":"conflict","message":"pubkey is backed by config (RELAY_OPERATOR_PUBKEYS or owner fallback) — immutable through the API"}}',
      status: 409,
      contains: "immutable through the API",
      excludes: "admin API error",
    },
    {
      name: "last-operator 409",
      pubkeyInput: "2a".repeat(32),
      message:
        'admin API error: {"error":{"code":"conflict","message":"operation would remove the last relay operator — add a replacement operator first"}}',
      status: 409,
      contains: "add a replacement operator first",
      excludes: "admin API error",
    },
    {
      name: "non-409 typed failure",
      pubkeyInput: "3b".repeat(32),
      message:
        'admin API error: {"error":{"code":"forbidden","message":"pubkey not permitted"}}',
      status: 403,
      contains: "pubkey not permitted",
      excludes: "admin API error",
    },
  ];

  let putResult = () => mutationReject(ADD_ERROR_ROWS[0].message, 409);
  setIpcHandler("admin_put_operator", () => putResult());

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey);
  await doRender();
  await settle(30);

  try {
    const pubkeyInput = container.querySelector(
      "[data-testid='staffing-add-pubkey-input']",
    );
    assert.ok(pubkeyInput, "pubkey input must be present");
    const addBtn = container.querySelector("[data-testid='staffing-add-btn']");
    assert.ok(addBtn, "Add button must be present");

    for (const row of ADD_ERROR_ROWS) {
      putResult = () => mutationReject(row.message, row.status);
      await act(async () => {
        fireEvent.change(pubkeyInput, { target: { value: row.pubkeyInput } });
        await new Promise((r) => setTimeout(r, 10));
      });
      await act(async () => {
        fireEvent.click(addBtn);
        await new Promise((r) => setTimeout(r, 30));
      });

      const errEls = Array.from(
        container.querySelectorAll(
          "[data-testid='staffing-tab'] .text-destructive",
        ),
      );
      assert.ok(
        errEls.some((el) => el.textContent.includes(row.contains)),
        `${row.name} must surface relay message containing "${row.contains}"; got: ${errEls.map((e) => e.textContent).join(", ")}`,
      );
      assert.ok(
        !errEls.some((el) => el.textContent.includes(row.excludes)),
        `${row.name} must not render the raw serialized error prefix`,
      );
    }
  } finally {
    await unmount();
  }
});

test("staffing-remove-409: deleteAdminOperator error cases surface the relay message", async () => {
  // handleConfirmRemove surfaces adminErrorMessage(e) for ALL errors — a 409
  // shows the relay's parsed message (config-backed OR last-operator conflict).
  //
  // Mutation evidence:
  //   - Restore the old adminMutationRelayStatus === 409 branch →
  //     last-operator row shows "config-backed" not the relay message → RED.
  //   - Remove adminErrorMessage(e) → raw JSON envelope renders → RED.
  //
  // Dialog confirmation is shared; cancel and pre-confirm zero-DELETE evidence
  // lives in staffing-remove-cancel and staffing-remove-confirm above.
  const origin = "https://admin-staffing-remove-reject.example.com";
  const pubkey = "07".repeat(32);
  const opPubkey = "1a".repeat(32);

  const REMOVE_ERROR_ROWS = [
    {
      name: "config-backed 409",
      message:
        'admin API error: {"error":{"code":"conflict","message":"pubkey is backed by config (RELAY_OPERATOR_PUBKEYS or owner fallback) — immutable through the API"}}',
      status: 409,
      contains: "immutable through the API",
      excludes: "admin API error",
    },
    {
      name: "last-operator 409",
      message:
        'admin API error: {"error":{"code":"conflict","message":"operation would remove the last relay operator — add a replacement operator first"}}',
      status: 409,
      contains: "add a replacement operator first",
      excludes: "admin API error",
    },
    {
      name: "non-409 typed failure",
      message:
        'admin API error: {"error":{"code":"internal","message":"operator store unavailable"}}',
      status: 500,
      contains: "operator store unavailable",
      excludes: "admin API error",
    },
  ];

  let deleteResult = () =>
    mutationReject(REMOVE_ERROR_ROWS[0].message, REMOVE_ERROR_ROWS[0].status);
  setIpcHandler("admin_delete_operator", () => deleteResult());

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey, [
    { pubkey: opPubkey, effectiveRole: "moderator", sources: ["db"] },
  ]);
  await doRender();
  await settle(30);

  const confirmRemove = async () => {
    const removeBtn = container.querySelector(
      `[data-testid='staffing-remove-btn-${opPubkey}']`,
    );
    assert.ok(removeBtn !== null, "remove button must be present");
    await act(async () => {
      fireEvent.click(removeBtn);
      await new Promise((r) => setTimeout(r, 10));
    });
    const confirmBtn = document.body.querySelector(
      "[data-testid='staffing-remove-confirm']",
    );
    assert.ok(confirmBtn !== null, "confirm button must be present in dialog");
    await act(async () => {
      fireEvent.click(confirmBtn);
      await new Promise((r) => setTimeout(r, 30));
    });
  };

  try {
    for (const row of REMOVE_ERROR_ROWS) {
      deleteResult = () => mutationReject(row.message, row.status);
      await confirmRemove();

      const errEls = Array.from(
        container.querySelectorAll(
          "[data-testid='staffing-tab'] [class*='destructive']",
        ),
      );
      assert.ok(
        errEls.some((el) => el.textContent.includes(row.contains)),
        `${row.name} must surface relay message containing "${row.contains}"; got: ${errEls.map((e) => e.textContent).join(", ")}`,
      );
      assert.ok(
        !errEls.some((el) => el.textContent.includes(row.excludes)),
        `${row.name} must not render the raw serialized error prefix`,
      );
    }
  } finally {
    await unmount();
  }
});

test("staffing-self-removal-fires-onSelfMutation: confirming removal of own pubkey calls onSelfMutation", async () => {
  // Verifies that handleConfirmRemove calls onSelfMutation when deleting the
  // current principal's own operator row.
  //
  // Without this callback the parent probe is never re-run after self-removal,
  // leaving the UI showing "Connected as operator" + Staffing tab even after
  // the operator has removed themselves.
  //
  // Mutation evidence:
  //   - Remove the `if (op.pubkey === pubkey) onSelfMutation?.()` guard →
  //     onSelfMutationCalls remains 0 → RED.
  const origin = "https://admin-staffing-self-remove.example.com";
  const pubkey = "ee".repeat(32); // self

  let onSelfMutationCalls = 0;

  setIpcHandler("admin_delete_operator", () => Promise.resolve());

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [{ pubkey: pubkey, effectiveRole: "operator", sources: ["db"] }],
    {
      onSelfMutation: () => {
        onSelfMutationCalls += 1;
      },
    },
  );
  await doRender();
  await settle(30);

  try {
    // Open confirmation dialog for self-removal
    const removeBtn = container.querySelector(
      `[data-testid='staffing-remove-btn-${pubkey}']`,
    );
    assert.ok(removeBtn !== null, "self remove button must be present");
    await act(async () => {
      fireEvent.click(removeBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const confirmBtn = document.body.querySelector(
      "[data-testid='staffing-remove-confirm']",
    );
    assert.ok(confirmBtn !== null, "confirm button must be present in dialog");
    await act(async () => {
      fireEvent.click(confirmBtn);
      await new Promise((r) => setTimeout(r, 30));
    });

    assert.equal(
      onSelfMutationCalls,
      1,
      `onSelfMutation must be called exactly once after self-removal; called ${onSelfMutationCalls} times`,
    );
  } finally {
    await unmount();
  }
});

// ── P1: Restrictions section ──────────────────────────────────────────────────
//
// The Restrictions section renders below the operator list for the active
// relay's community. It lists active bans/timeouts and provides per-row Lift ban /
// Clear timeout buttons with confirmation dialogs.
//
// Mutation evidence:
//   - Remove confirm dialog → lift IPC fires on button click without confirm → RED.
//   - Remove the list refresh after lift → row stays after lift → RED.

function makeBanRecord(pubkeyHex, overrides = {}) {
  return {
    pubkey: pubkeyHex,
    banned: true,
    banExpiresAt: null,
    banReason: "test ban",
    mutedUntil: null,
    muteReason: null,
    actorPubkey: "aa".repeat(32),
    updatedAt: new Date().toISOString(),
    ...overrides,
  };
}

function makeTimeoutRecord(pubkeyHex, overrides = {}) {
  // mutedUntil 1 hour in the future
  const future = new Date(Date.now() + 3_600_000).toISOString();
  return {
    pubkey: pubkeyHex,
    banned: false,
    banExpiresAt: null,
    banReason: null,
    mutedUntil: future,
    muteReason: "test timeout",
    actorPubkey: "aa".repeat(32),
    updatedAt: new Date().toISOString(),
    ...overrides,
  };
}

test("restrictions-empty: restrictions section shows 'no active bans or timeouts' when list is empty", async () => {
  const origin = "https://admin-restrictions.example.com";
  const pubkey = "a1".repeat(32);

  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({ items: [], nextCursor: null }),
  );

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    const section = container.querySelector(
      "[data-testid='restrictions-section']",
    );
    assert.ok(section !== null, "restrictions-section must render");
    const emptyMsg = container.querySelector(
      "[data-testid='restrictions-empty']",
    );
    assert.ok(
      emptyMsg !== null,
      "restrictions-empty must render when list is empty",
    );
    assert.ok(
      emptyMsg.textContent.includes("No active bans"),
      `empty message must mention "No active bans"; got: "${emptyMsg.textContent}"`,
    );
  } finally {
    await unmount();
  }
});

test("restrictions-unknown-host-is-an-error: an unresolved community host shows the error, never the empty state", async () => {
  // The relay rejects a host it serves no community for with
  // unknown_community_host; the list must say so rather than claim there are
  // no active bans.
  //
  // Mutation evidence: render the empty state on error → RED.
  const origin = "https://admin-restrictions-unknown.example.com";
  const pubkey = "b2".repeat(32);

  const listCalls = [];
  setIpcHandler("admin_list_restrictions", (args) => {
    listCalls.push(args);
    return Promise.reject(
      'admin relay error 400: {"error":"unknown_community_host","message":"no community is served at this host"}',
    );
  });

  const { container, doRender, unmount } = mountStaffingPanel(origin, pubkey);
  await doRender();
  await settle(30);

  try {
    assert.ok(
      container.querySelector("[data-testid='restrictions-section']"),
      "restrictions-section must render",
    );
    assert.equal(
      container.querySelector("[data-testid='restrictions-empty']"),
      null,
      "an unresolved host must not render as 'No active bans or timeouts'",
    );
    assert.ok(
      container.textContent.includes("no community is served at this host") ||
        container.textContent.includes("unknown_community_host"),
      `error must be shown; got: ${container.textContent}`,
    );
    assert.equal(listCalls.length >= 1, true);
    assert.equal(
      "communityId" in listCalls[0],
      false,
      "no client community id is sent",
    );
  } finally {
    await unmount();
  }
});

test("restrictions-rows: banned and timed-out members render with correct buttons", async () => {
  const origin = "https://admin-restrictions-rows.example.com";
  const pubkey = "c3".repeat(32);
  const bannedPubkey = "d4".repeat(32);
  const timedOutPubkey = "e5".repeat(32);

  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({
      items: [makeBanRecord(bannedPubkey), makeTimeoutRecord(timedOutPubkey)],
      nextCursor: null,
    }),
  );

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    // Banned row
    const banRow = container.querySelector(
      `[data-testid='restriction-row-${bannedPubkey}']`,
    );
    assert.ok(banRow !== null, "banned member row must render");
    const liftBanBtn = container.querySelector(
      `[data-testid='restrictions-lift-ban-btn-${bannedPubkey}']`,
    );
    assert.ok(
      liftBanBtn !== null,
      "Lift ban button must be present for a banned member",
    );

    // Timed-out row
    const timeoutRow = container.querySelector(
      `[data-testid='restriction-row-${timedOutPubkey}']`,
    );
    assert.ok(timeoutRow !== null, "timed-out member row must render");
    const clearTimeoutBtn = container.querySelector(
      `[data-testid='restrictions-lift-timeout-btn-${timedOutPubkey}']`,
    );
    assert.ok(
      clearTimeoutBtn !== null,
      "Clear timeout button must be present for a timed-out member",
    );

    // Banned member must NOT have a clear-timeout button
    const noClearBtn = container.querySelector(
      `[data-testid='restrictions-lift-timeout-btn-${bannedPubkey}']`,
    );
    assert.equal(
      noClearBtn,
      null,
      "Clear timeout button must be absent for a banned-only member",
    );
  } finally {
    await unmount();
  }
});

test("restrictions-lift-ban-cancel: cancel does not invoke admin_lift_ban", async () => {
  const origin = "https://admin-restrictions-cancel-ban.example.com";
  const pubkey = "f6".repeat(32);
  const bannedPubkey = "07".repeat(32);

  const liftCalls = [];
  setIpcHandler("admin_lift_ban", (args) => {
    liftCalls.push(args);
    return Promise.resolve();
  });
  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({ items: [makeBanRecord(bannedPubkey)], nextCursor: null }),
  );

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    const liftBanBtn = container.querySelector(
      `[data-testid='restrictions-lift-ban-btn-${bannedPubkey}']`,
    );
    assert.ok(liftBanBtn !== null, "lift ban button must be present");
    await act(async () => {
      fireEvent.click(liftBanBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const dialog = document.body.querySelector(
      "[data-testid='restrictions-lift-ban-dialog']",
    );
    assert.ok(dialog !== null, "lift-ban dialog must open");
    assert.equal(
      liftCalls.length,
      0,
      "admin_lift_ban must not fire before confirm",
    );

    const cancelBtn = document.body.querySelector(
      "[data-testid='restrictions-lift-ban-cancel']",
    );
    assert.ok(cancelBtn !== null, "cancel button must be present");
    await act(async () => {
      fireEvent.click(cancelBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    assert.equal(
      liftCalls.length,
      0,
      "admin_lift_ban must not fire after cancel",
    );
    const dialogAfter = document.body.querySelector(
      "[data-testid='restrictions-lift-ban-dialog']",
    );
    assert.equal(dialogAfter, null, "dialog must close after cancel");
  } finally {
    await unmount();
  }
});

test("restrictions-lift-ban-confirm: confirming lift-ban calls admin_lift_ban with correct args and refreshes list", async () => {
  //
  // Mutation evidence:
  //   - Remove the handleConfirmLiftBan → liftBan call → liftCalls stays 0 → RED.
  //   - Remove setListGen bump → row stays after lift → RED (list not refreshed).
  const origin = "https://admin-restrictions-confirm-ban.example.com";
  const pubkey = "18".repeat(32);
  const bannedPubkey = "29".repeat(32);

  const liftCalls = [];
  let remainingItems = [makeBanRecord(bannedPubkey)];

  setIpcHandler("admin_lift_ban", (args) => {
    liftCalls.push(args);
    remainingItems = [];
    return Promise.resolve();
  });
  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({ items: [...remainingItems], nextCursor: null }),
  );

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    const liftBanBtn = container.querySelector(
      `[data-testid='restrictions-lift-ban-btn-${bannedPubkey}']`,
    );
    assert.ok(
      liftBanBtn !== null,
      "lift ban button must be present before confirm",
    );

    await act(async () => {
      fireEvent.click(liftBanBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const confirmBtn = document.body.querySelector(
      "[data-testid='restrictions-lift-ban-confirm']",
    );
    assert.ok(confirmBtn !== null, "confirm button must be present in dialog");
    await act(async () => {
      fireEvent.click(confirmBtn);
      await new Promise((r) => setTimeout(r, 50));
    });

    assert.equal(
      liftCalls.length,
      1,
      `admin_lift_ban must be called exactly once; got ${liftCalls.length}`,
    );
    assert.equal(
      liftCalls[0]?.pubkey,
      bannedPubkey,
      `admin_lift_ban must receive the banned pubkey; got: ${liftCalls[0]?.pubkey}`,
    );
    assert.equal(
      "communityId" in (liftCalls[0] ?? {}),
      false,
      "admin_lift_ban must not send a client community id; the native command derives the relay host",
    );
    assert.equal(
      liftCalls[0]?.expectedRelay,
      TEST_RELAY_WS_URL,
      "admin_lift_ban carries the relay the list loaded from",
    );

    // After the lift the list refreshes and the row must be gone.
    await settle(50);
    const rowAfter = container.querySelector(
      `[data-testid='restriction-row-${bannedPubkey}']`,
    );
    assert.equal(
      rowAfter,
      null,
      "banned member row must be gone after ban is lifted",
    );
  } finally {
    await unmount();
  }
});

test("restrictions-lift-timeout-confirm: confirming clear-timeout calls admin_lift_timeout with correct args", async () => {
  const origin = "https://admin-restrictions-confirm-timeout.example.com";
  const pubkey = "3a".repeat(32);
  const timedOutPubkey = "4b".repeat(32);

  const liftCalls = [];
  let remainingItems = [makeTimeoutRecord(timedOutPubkey)];

  setIpcHandler("admin_lift_timeout", (args) => {
    liftCalls.push(args);
    remainingItems = [];
    return Promise.resolve();
  });
  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({ items: [...remainingItems], nextCursor: null }),
  );

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    const clearTimeoutBtn = container.querySelector(
      `[data-testid='restrictions-lift-timeout-btn-${timedOutPubkey}']`,
    );
    assert.ok(clearTimeoutBtn !== null, "clear timeout button must be present");

    await act(async () => {
      fireEvent.click(clearTimeoutBtn);
      await new Promise((r) => setTimeout(r, 10));
    });

    const confirmBtn = document.body.querySelector(
      "[data-testid='restrictions-lift-timeout-confirm']",
    );
    assert.ok(confirmBtn !== null, "confirm button must be present in dialog");
    await act(async () => {
      fireEvent.click(confirmBtn);
      await new Promise((r) => setTimeout(r, 50));
    });

    assert.equal(
      liftCalls.length,
      1,
      `admin_lift_timeout must be called exactly once; got ${liftCalls.length}`,
    );
    assert.equal(
      liftCalls[0]?.pubkey,
      timedOutPubkey,
      `admin_lift_timeout must receive the timed-out pubkey; got: ${liftCalls[0]?.pubkey}`,
    );
    assert.equal(
      "communityId" in (liftCalls[0] ?? {}),
      false,
      "admin_lift_timeout must not send a client community id; the native command derives the relay host",
    );

    // Row must be gone after list refresh.
    await settle(50);
    const rowAfter = container.querySelector(
      `[data-testid='restriction-row-${timedOutPubkey}']`,
    );
    assert.equal(
      rowAfter,
      null,
      "timed-out member row must be gone after timeout is cleared",
    );
  } finally {
    await unmount();
  }
});

test("restrictions-lift-409-treated-as-success: a 409 (already gone) refreshes the list without showing an error", async () => {
  // When the relay returns 409 ("no active ban"), the row is already gone on
  // the server. The UI treats this as a soft success: refresh the list,
  // don\'t surface an error.
  //
  // Mutation evidence:
  //   - Remove the 409-as-success catch branch → liftError set → errEl found → RED.
  //   - Remove setListGen → row stays after lift → row visible → can assert RED
  //     by checking the ban row is absent (or use restrictions-lift-ban-confirm
  //     which already covers the setListGen call on success).
  const origin = "https://admin-restrictions-409.example.com";
  const pubkey = "5c".repeat(32);
  const bannedPubkey = "6d".repeat(32);

  // `liftAttempted` flips to true only after admin_lift_ban is invoked, so the
  // subsequent list refresh (setListGen inside catch) returns an empty list.
  // Using a flag instead of a counter avoids races from multiple initial loads
  // (AdminConsolePanel\'s generation-bump useEffect causes 2 loads on mount).
  let liftAttempted = false;
  setIpcHandler("admin_lift_ban", () => {
    liftAttempted = true;
    return mutationReject(
      'admin API error: {"error":{"code":"conflict","message":"no active ban for this member"}}',
      409,
    );
  });
  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({
      // Before lift attempt: show the row. After lift attempt: empty (gone).
      items: liftAttempted ? [] : [makeBanRecord(bannedPubkey)],
      nextCursor: null,
    }),
  );

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    const liftBanBtn = container.querySelector(
      `[data-testid='restrictions-lift-ban-btn-${bannedPubkey}']`,
    );
    assert.ok(liftBanBtn !== null, "lift ban button must be present");

    await act(async () => {
      fireEvent.click(liftBanBtn);
      await new Promise((r) => setTimeout(r, 10));
    });
    const confirmBtn = document.body.querySelector(
      "[data-testid='restrictions-lift-ban-confirm']",
    );
    assert.ok(confirmBtn !== null, "confirm button must be present");
    await act(async () => {
      fireEvent.click(confirmBtn);
      await new Promise((r) => setTimeout(r, 50));
    });

    // No error must be visible — 409 is a soft success.
    const errEl = container.querySelector(
      "[data-testid='restrictions-section'] [class*='destructive']",
    );
    assert.equal(
      errEl,
      null,
      "no error must be shown when 409 (already gone) is returned",
    );
    // Row must be gone (setListGen triggered a refresh which returned empty).
    assert.ok(liftAttempted, "admin_lift_ban must have been called");
    const rowAfter = container.querySelector(
      `[data-testid='restriction-row-${bannedPubkey}']`,
    );
    assert.equal(
      rowAfter,
      null,
      "ban row must be absent after soft-success refresh",
    );
  } finally {
    await unmount();
  }
});

test("restrictions-load-more: second page is fetched with the cursor and appended", async () => {
  const origin = "https://admin-restrictions-pages.example.com";
  const pubkey = "c3".repeat(32);
  const firstPubkey = "d4".repeat(32);
  const secondPubkey = "e5".repeat(32);
  const cursors = [];
  const relays = [];

  setIpcHandler("admin_list_restrictions", (args) => {
    cursors.push(args?.cursor ?? null);
    relays.push(args?.expectedRelay);
    return Promise.resolve(
      args?.cursor === "page-2"
        ? { items: [makeBanRecord(secondPubkey)], nextCursor: null }
        : { items: [makeBanRecord(firstPubkey)], nextCursor: "page-2" },
    );
  });

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    const row = (pk) =>
      container.querySelector(`[data-testid='restriction-row-${pk}']`);
    const loadMore = () =>
      container.querySelector("[data-testid='restrictions-load-more']");
    assert.ok(row(firstPubkey), "first page row must render");
    assert.equal(row(secondPubkey), null, "second page is not loaded yet");
    assert.ok(loadMore(), "Load more must show while nextCursor is set");

    await act(async () => {
      fireEvent.click(loadMore());
    });
    await settle(50);

    assert.equal(cursors.at(-1), "page-2", "Load more forwards nextCursor");
    assert.equal(cursors.filter((c) => c !== null).length, 1);
    assert.deepEqual(
      [...new Set(relays)],
      [TEST_RELAY_WS_URL],
      "every page carries the relay the list loaded from",
    );
    assert.ok(row(firstPubkey), "first page row stays");
    assert.ok(row(secondPubkey), "second page row is appended");
    assert.ok(
      container.querySelector(
        `[data-testid='restrictions-lift-ban-btn-${secondPubkey}']`,
      ),
      "a second-page member can be lifted",
    );
    assert.equal(loadMore(), null, "Load more hides on the last page");
  } finally {
    await unmount();
  }
});

test("restrictions-load-more-stale-error: a failed old page does not survive a successful lift/reload", async () => {
  const origin = "https://review-lift-fence.example.com",
    pubkey = "ab".repeat(32),
    pk = "cd".repeat(32);
  let rejectPage,
    lifted = false;
  const pending = new Promise((_, j) => {
    rejectPage = j;
  });
  setIpcHandler("admin_list_restrictions", (args) =>
    args.cursor
      ? pending
      : Promise.resolve({
          items: lifted ? [] : [makeBanRecord(pk)],
          nextCursor: lifted ? null : "next",
        }),
  );
  setIpcHandler("admin_lift_ban", () => {
    lifted = true;
    return Promise.resolve();
  });
  const m = mountStaffingPanel(origin, pubkey, [], {});
  try {
    await m.doRender();
    await settle(30);
    await act(async () =>
      fireEvent.click(
        m.container.querySelector("[data-testid='restrictions-load-more']"),
      ),
    );
    await act(async () =>
      fireEvent.click(
        m.container.querySelector(
          `[data-testid='restrictions-lift-ban-btn-${pk}']`,
        ),
      ),
    );
    await act(async () =>
      fireEvent.click(
        document.body.querySelector(
          "[data-testid='restrictions-lift-ban-confirm']",
        ),
      ),
    );
    await settle(30);
    await act(async () => rejectPage(new Error("OBSOLETE_PAGE_FAILED")));
    await settle(30);
    assert.ok(
      m.container.querySelector("[data-testid='restrictions-empty']"),
      "successful fresh list is displayed",
    );
    assert.ok(
      !m.container.textContent.includes("OBSOLETE_PAGE_FAILED"),
      "old page error must not contaminate refreshed list",
    );
  } finally {
    await m.unmount();
  }
});

test("restrictions-relay-changed: a removal rejected for a changed relay shows the error and keeps the row", async () => {
  // The native command rejects when the active relay no longer matches the
  // relay the list loaded from; the UI must say so, not silently drop the row.
  const origin = "https://admin-restrictions-relay-changed.example.com";
  const pubkey = "3a".repeat(32);
  const bannedPubkey = "4b".repeat(32);
  const scopeError =
    "active community changed since restrictions loaded; nothing was sent. Reload to continue.";

  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({ items: [makeBanRecord(bannedPubkey)], nextCursor: null }),
  );
  setIpcHandler("admin_lift_ban", () => mutationReject(scopeError, null));

  const { container, doRender, unmount } = mountStaffingPanel(
    origin,
    pubkey,
    [],
  );
  await doRender();
  await settle(50);

  try {
    await act(async () => {
      fireEvent.click(
        container.querySelector(
          `[data-testid='restrictions-lift-ban-btn-${bannedPubkey}']`,
        ),
      );
      await new Promise((r) => setTimeout(r, 10));
    });
    await act(async () => {
      fireEvent.click(
        document.body.querySelector(
          "[data-testid='restrictions-lift-ban-confirm']",
        ),
      );
      await new Promise((r) => setTimeout(r, 50));
    });
    assert.ok(
      container.textContent.includes("active community changed"),
      `scope error must be shown; got: ${container.textContent}`,
    );
    assert.ok(
      container.querySelector(
        `[data-testid='restriction-row-${bannedPubkey}']`,
      ),
      "the row stays: nothing was lifted",
    );
  } finally {
    await unmount();
  }
});
