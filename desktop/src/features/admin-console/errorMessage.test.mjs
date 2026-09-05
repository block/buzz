/**
 * Unit tests for adminErrorMessage — the parser that turns a native admin
 * mutation rejection into the human-readable text surfaced via toast.error.
 *
 * Native admin commands reject with `admin API error: {json}` where the JSON
 * is the relay's error envelope. The parser strips the prefix and returns the
 * envelope's `message`, falling back to the raw string for anything that is
 * not that shape (network errors, plain strings, malformed JSON).
 */

import assert from "node:assert/strict";
import test from "node:test";

import { adminErrorMessage } from "./AdminConsolePanelHelpers.tsx";

test("extracts-envelope-message: returns the relay error message, not the raw JSON", () => {
  const raw =
    'admin API error: {"error":{"code":"invalid_action_for_target","message":"action kick requires the report to have an associated channel","requestId":"abc"}}';
  assert.equal(
    adminErrorMessage(new Error(raw)),
    "action kick requires the report to have an associated channel",
  );
});

test("accepts-raw-string-input: parses when passed a string rather than an Error", () => {
  const raw =
    'admin API error: {"error":{"message":"report is not open (current status: processing)"}}';
  assert.equal(
    adminErrorMessage(raw),
    "report is not open (current status: processing)",
  );
});

test("falls-back-on-non-json: a plain network error returns its raw text", () => {
  assert.equal(
    adminErrorMessage(new Error("Failed to fetch")),
    "Failed to fetch",
  );
});

test("falls-back-on-malformed-json: an unparseable brace payload returns raw", () => {
  const raw = "admin API error: {not valid json";
  assert.equal(adminErrorMessage(new Error(raw)), raw);
});

test("falls-back-on-empty-message: an envelope with a blank message returns raw", () => {
  const raw = 'admin API error: {"error":{"code":"x","message":""}}';
  assert.equal(adminErrorMessage(new Error(raw)), raw);
});

test("falls-back-on-absent-message: an envelope without a message field returns raw", () => {
  const raw = 'admin API error: {"error":{"code":"internal"}}';
  assert.equal(adminErrorMessage(new Error(raw)), raw);
});
