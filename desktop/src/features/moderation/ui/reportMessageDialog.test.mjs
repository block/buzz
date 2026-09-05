/**
 * Unit tests for reportErrorMessage (item 8 — report-failure UX).
 *
 * Verifies that the relay's own error message is surfaced instead of the
 * generic "Failed to submit report" string. Also covers the status-prefix
 * stripping so raw HTTP status codes don't appear in the toast.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { reportErrorMessage } from "./ReportMessageDialog.tsx";

test("surfaces-relay-message: returns relay error text stripped of status prefix", () => {
  // A relay rejection arrives as "400: you cannot report your own message".
  assert.equal(
    reportErrorMessage(new Error("400: you cannot report your own message")),
    "you cannot report your own message",
  );
});

test("surfaces-relay-message-no-prefix: relay error without prefix returned verbatim", () => {
  assert.equal(
    reportErrorMessage(new Error("report is invalid: missing event id")),
    "report is invalid: missing event id",
  );
});

test("fallback-empty-stripped: empty message after stripping returns generic string", () => {
  // "400: " strips to "" → fallback.
  assert.equal(
    reportErrorMessage(new Error("400: ")),
    "Failed to submit report",
  );
});

test("fallback-non-error: non-Error input returns generic string", () => {
  assert.equal(reportErrorMessage(null), "Failed to submit report");
  assert.equal(reportErrorMessage(undefined), "Failed to submit report");
  assert.equal(reportErrorMessage("bare string"), "Failed to submit report");
});

test("fallback-empty-error-message: Error with empty message returns generic string", () => {
  assert.equal(reportErrorMessage(new Error("")), "Failed to submit report");
});
