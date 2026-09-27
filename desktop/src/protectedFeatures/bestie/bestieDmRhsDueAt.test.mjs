import assert from "node:assert/strict";
import test from "node:test";

import {
  datetimeLocalFromDueAt,
  dueAtFromDatetimeLocal,
} from "./bestieDmRhsHelpers.ts";

test("datetime-local round-trip", () => {
  const dueAt = dueAtFromDatetimeLocal("2026-09-26T16:30");
  assert.ok(dueAt);
  const local = datetimeLocalFromDueAt(dueAt);
  assert.match(local, /^2026-09-26T16:30$/);
  assert.equal(dueAtFromDatetimeLocal(""), null);
  assert.equal(dueAtFromDatetimeLocal("not-a-date"), null);
});
