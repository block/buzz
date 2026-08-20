import assert from "node:assert/strict";
import test from "node:test";

import { waitAge } from "./WorkstreamWaits.tsx";

const now = Date.parse("2026-08-19T16:00:00Z");

test("waitAge labels age unknown when no trustworthy timestamp exists", () => {
  assert.equal(waitAge(undefined, now), "Unknown");
  assert.equal(waitAge("not-a-timestamp", now), "Unknown");
});

test("waitAge reports elapsed age for a trustworthy timestamp", () => {
  assert.equal(waitAge("2026-08-19T15:59:30Z", now), "0m");
  assert.equal(waitAge("2026-08-19T14:00:00Z", now), "2h");
});
