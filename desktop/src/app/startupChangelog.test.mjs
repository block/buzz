import assert from "node:assert/strict";
import test from "node:test";

import { recentStartupChangelog } from "./startupChangelog.ts";

test("keeps the newest ten calendar days and sorts newest first", () => {
  const entries = [
    { date: "2026-08-16", items: ["too old"] },
    { date: "2026-08-26", items: ["newest"] },
    { date: "2026-08-17", items: ["boundary"] },
    { date: "2026-08-20", items: ["middle"] },
  ];

  assert.deepEqual(
    recentStartupChangelog(entries).map((entry) => entry.date),
    ["2026-08-26", "2026-08-20", "2026-08-17"],
  );
});

test("uses the newest log date instead of the current computer date", () => {
  assert.deepEqual(
    recentStartupChangelog([
      { date: "2020-01-01", items: ["old build"] },
      { date: "2019-12-31", items: ["still visible"] },
    ]).map((entry) => entry.date),
    ["2020-01-01", "2019-12-31"],
  );
});

test("returns no entries for a disabled window", () => {
  assert.deepEqual(
    recentStartupChangelog([{ date: "2026-08-26", items: ["item"] }], 0),
    [],
  );
});
