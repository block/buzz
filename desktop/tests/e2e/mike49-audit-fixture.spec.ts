/**
 * MIKE-49 checkpoint 1: BROWSER evidence, with `@tauri-apps/api/mocks`'
 * `mockIPC` (wired in `src/testing/e2eBridge.ts`) substituted for
 * `window.__TAURI_INTERNALS__.invoke` — this proves the frontend's click ->
 * `invoke()` call -> render path (explicit-run/loading/success/error/
 * duplicate-run-guard states) against a JS-only canned response for
 * `mike49_run_fixture_audit`. It does NOT execute the compiled Rust
 * command, and it does NOT exercise Tauri's real IPC (de)serialization
 * into a webview — `mockIPC` intercepts the call before either happens.
 * No real identity, relay, or archive is reachable from this test either
 * way: `installMockBridge` never connects to a live relay, and the real
 * command itself takes no `AppState` (see
 * `desktop/src-tauri/src/commands/mike49_audit/command.rs`'s own module
 * doc comment).
 *
 * Compiled Rust command dispatch and its serialization into a (mock)
 * webview are proven separately, via real `tauri::test::get_ipc_response`
 * IPC dispatch with no browser involved, in
 * `desktop/src-tauri/src/commands/mike49_audit/command.rs`'s
 * `ipc_tests::native_ipc_round_trip_produces_the_same_report_shape_as_a_direct_call`.
 * The Rust fixture/sanitizer logic itself (including
 * `record_sanitizer_matches_the_pinned_python_reference_on_shared_examples`)
 * is covered by that crate's own `cargo test` suite. Neither of those is a
 * compiled, launched Tauri application with a real native window — a
 * human visual click-through of the actual app remains the only step this
 * checkpoint has not exercised.
 */

import { expect, test, type Page } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

async function openMike49Panel(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-experimental").click();
  await expect(page.getByTestId("mike49-audit-panel")).toBeVisible({
    timeout: 10_000,
  });
  await waitForAnimations(page);
}

test("explicit run: idle -> loading -> success renders the fixture report", async ({
  page,
}) => {
  await installMockBridge(page, { mike49AuditDelayMs: 300 });
  await openMike49Panel(page);

  const runButton = page.getByTestId("mike49-run-button");
  await expect(page.getByTestId("mike49-loading")).not.toBeVisible();
  await expect(page.getByTestId("mike49-report")).not.toBeVisible();

  await runButton.click();

  // Loading state is observable because the mock response is delayed.
  await expect(page.getByTestId("mike49-loading")).toBeVisible();
  await expect(runButton).toBeDisabled();

  await expect(page.getByTestId("mike49-report")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("mike49-loading")).not.toBeVisible();
  await expect(runButton).toBeEnabled();

  // Two records: one verified, one verify_error -- matches the real
  // fixture's `accepted_record` / `wrong_recipient_verify_error` scenarios.
  const records = page.getByTestId("mike49-record-row");
  await expect(records).toHaveCount(2);
  await expect(records.nth(0)).toContainText("accepted_record");
  await expect(records.nth(0)).toContainText("Verified");
  await expect(records.nth(1)).toContainText("wrong_recipient_verify_error");
  await expect(records.nth(1)).toContainText("Verify error");
  await expect(records.nth(1)).toContainText("wrong_recipient");

  // Capped/incomplete pagination is surfaced, not hidden.
  await expect(page.getByTestId("mike49-pagination-state")).toContainText(
    "Incomplete",
  );
  await expect(page.getByTestId("mike49-empty-probe")).toContainText(
    "0 events",
  );
  await expect(page.getByTestId("mike49-report")).toContainText(
    "SIMULATED DATA",
  );
});

test("failure state: a rejected invoke shows the fixed, content-free error copy", async ({
  page,
}) => {
  await installMockBridge(page, { mike49AuditError: "reconciliation_failed" });
  await openMike49Panel(page);

  await page.getByTestId("mike49-run-button").click();

  await expect(page.getByTestId("mike49-error")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("mike49-error")).toContainText(
    "did not reconcile",
  );
  await expect(page.getByTestId("mike49-report")).not.toBeVisible();
  await expect(page.getByTestId("mike49-run-button")).toBeEnabled();
});

test("duplicate-run guard: a second click during loading does not start a second run", async ({
  page,
}) => {
  await installMockBridge(page, { mike49AuditDelayMs: 500 });
  await openMike49Panel(page);

  await page.evaluate(() => {
    const w = window as typeof window & {
      __mike49InvokeCount?: number;
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload: unknown,
          options: unknown,
        ) => Promise<unknown>;
      };
    };
    w.__mike49InvokeCount = 0;
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = (command, payload, options) => {
      if (command === "mike49_run_fixture_audit") {
        w.__mike49InvokeCount = (w.__mike49InvokeCount ?? 0) + 1;
      }
      return original(command, payload, options);
    };
  });

  const runButton = page.getByTestId("mike49-run-button");
  await runButton.click();
  await expect(page.getByTestId("mike49-loading")).toBeVisible();

  // The button is `disabled` while loading, so a real Playwright `.click()`
  // would refuse (actionability check) -- this exercises the ref-guard
  // itself (see `Mike49AuditFixturePanel.tsx`'s `isRunningRef` comment) via
  // a programmatic dispatch that bypasses the disabled-button UI guard.
  await page.evaluate(() => {
    document
      .querySelector('[data-testid="mike49-run-button"]')
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });

  await expect(page.getByTestId("mike49-report")).toBeVisible({
    timeout: 10_000,
  });

  const count = await page.evaluate(() => {
    const w = window as typeof window & { __mike49InvokeCount?: number };
    return w.__mike49InvokeCount ?? 0;
  });
  expect(count).toBe(1);
});
