import assert from "node:assert/strict";
import { test } from "node:test";

import {
  compareVersions,
  coreVersionOf,
  localReleaseRows,
  mergeReleaseTimeline,
  parseUpstreamReleases,
} from "./upstreamReleases.mjs";

/** Minimal GitHub release payload entry. */
function release(tag, overrides = {}) {
  return {
    tag_name: tag,
    published_at: "2026-08-10T12:00:00Z",
    html_url: `https://github.com/block/buzz/releases/tag/${tag}`,
    draft: false,
    prerelease: false,
    ...overrides,
  };
}

test("compareVersions orders by numeric segment, not string", () => {
  // "0.5.14" < "0.5.5" as strings; the whole point is that it must not be.
  assert.ok(compareVersions("0.5.14", "0.5.5") > 0);
  assert.ok(compareVersions("0.5.5", "0.5.14") < 0);
  assert.equal(compareVersions("0.5.0", "0.5"), 0);
  assert.equal(compareVersions("1.2.3", "1.2.3"), 0);
});

test("compareVersions ignores any prerelease suffix", () => {
  assert.equal(compareVersions("0.5.14-2", "0.5.14"), 0);
});

test("coreVersionOf strips the k2alpha counter", () => {
  assert.equal(coreVersionOf("0.5.14-2"), "0.5.14");
  assert.equal(coreVersionOf("0.5.14"), "0.5.14");
  assert.equal(coreVersionOf("nonsense"), null);
  assert.equal(coreVersionOf(undefined), null);
});

test("only desktop tags are kept", () => {
  // Upstream ships relay and chart releases from the same repo.
  const rows = parseUpstreamReleases(
    [
      release("desktop-v0.5.14"),
      release("relay-v2.0.0"),
      release("chart-v0.1.0"),
    ],
    "0.5.14",
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].version, "0.5.14");
});

test("releases newer than the merged version are excluded", () => {
  // The whole reason for the ceiling: these describe features this build
  // does not have.
  const rows = parseUpstreamReleases(
    [
      release("desktop-v0.5.13"),
      release("desktop-v0.5.14"),
      release("desktop-v0.5.15"),
      release("desktop-v0.6.0"),
    ],
    "0.5.14",
  );
  assert.deepEqual(
    rows.map((row) => row.version),
    ["0.5.13", "0.5.14"],
  );
});

test("no ceiling keeps everything", () => {
  const rows = parseUpstreamReleases(
    [release("desktop-v0.5.15"), release("desktop-v9.9.9")],
    null,
  );
  assert.equal(rows.length, 2);
});

test("drafts and prereleases are excluded", () => {
  const rows = parseUpstreamReleases(
    [
      release("desktop-v0.5.10", { draft: true }),
      release("desktop-v0.5.11", { prerelease: true }),
      release("desktop-v0.5.12"),
    ],
    "0.5.14",
  );
  assert.deepEqual(
    rows.map((row) => row.version),
    ["0.5.12"],
  );
});

test("the date is reduced to a plain day", () => {
  const rows = parseUpstreamReleases(
    [release("desktop-v0.5.14", { published_at: "2026-08-15T09:30:00Z" })],
    "0.5.14",
  );
  assert.equal(rows[0].date, "2026-08-15");
});

test("malformed payloads produce no rows rather than throwing", () => {
  // This is a third-party API response; none of its shape is guaranteed.
  assert.deepEqual(parseUpstreamReleases(null, "0.5.14"), []);
  assert.deepEqual(parseUpstreamReleases({}, "0.5.14"), []);
  assert.deepEqual(parseUpstreamReleases("nope", "0.5.14"), []);
  assert.deepEqual(
    parseUpstreamReleases(
      [null, {}, { tag_name: 42 }, release("desktop-vNaN")],
      "0.5.14",
    ),
    [],
  );
});

test("a missing published_at leaves the date null, not invalid", () => {
  const rows = parseUpstreamReleases(
    [release("desktop-v0.5.14", { published_at: null })],
    "0.5.14",
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].date, null);
});

test("local rows carry their bullets and tolerate missing dates", () => {
  const rows = localReleaseRows([
    { version: "0.5.14-2", date: "2026-08-16", bullets: ["a"] },
    { version: "0.5.5-2", bullets: ["b"] },
    null,
  ]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].source, "local");
  assert.deepEqual(rows[0].bullets, ["a"]);
  assert.equal(rows[1].date, null);
});

test("the merged timeline runs newest first", () => {
  const merged = mergeReleaseTimeline(
    localReleaseRows([
      { version: "0.5.14-2", date: "2026-08-16", bullets: [] },
      { version: "0.5.5-4", date: "2026-08-12", bullets: [] },
    ]),
    parseUpstreamReleases(
      [
        release("desktop-v0.5.14", { published_at: "2026-08-14T00:00:00Z" }),
        release("desktop-v0.5.13", { published_at: "2026-08-01T00:00:00Z" }),
      ],
      "0.5.14",
    ),
  );

  assert.deepEqual(
    merged.map((row) => row.date),
    ["2026-08-16", "2026-08-14", "2026-08-12", "2026-08-01"],
  );
});

test("two releases on the same day run newest first", () => {
  // The regression: compareVersions strips the prerelease suffix, so
  // "0.5.14-4" and "0.5.14-5" compare equal and the stable sort left them in
  // array order — oldest first, which is backwards.
  const merged = mergeReleaseTimeline(
    localReleaseRows([
      { version: "0.5.14-4", date: "2026-08-17", bullets: [] },
      { version: "0.5.14-5", date: "2026-08-17", bullets: [] },
    ]),
    [],
  );

  assert.deepEqual(
    merged.map((row) => row.version),
    ["0.5.14-5", "0.5.14-4"],
  );
});

test("a whole same-day changelog stays in release order", () => {
  const merged = mergeReleaseTimeline(
    localReleaseRows([
      { version: "0.5.14-0", date: "2026-08-15", bullets: [] },
      { version: "0.5.14-1", date: "2026-08-15", bullets: [] },
      { version: "0.5.14-2", date: "2026-08-16", bullets: [] },
      { version: "0.5.14-3", date: "2026-08-16", bullets: [] },
      { version: "0.5.14-4", date: "2026-08-17", bullets: [] },
      { version: "0.5.14-5", date: "2026-08-17", bullets: [] },
    ]),
    [],
  );

  assert.deepEqual(
    merged.map((row) => row.version),
    ["0.5.14-5", "0.5.14-4", "0.5.14-3", "0.5.14-2", "0.5.14-1", "0.5.14-0"],
  );
});

test("undated rows sort last, never first", () => {
  const merged = mergeReleaseTimeline(
    localReleaseRows([
      { version: "0.5.5-2", bullets: [] },
      { version: "0.5.14-2", date: "2026-08-16", bullets: [] },
    ]),
    [],
  );
  assert.equal(merged[0].version, "0.5.14-2");
  assert.equal(merged[1].version, "0.5.5-2");
});

test("same-day entries put the fork's release above upstream's, stably", () => {
  const build = () =>
    mergeReleaseTimeline(
      localReleaseRows([
        { version: "0.5.14-0", date: "2026-08-15", bullets: [] },
      ]),
      parseUpstreamReleases(
        [release("desktop-v0.5.14", { published_at: "2026-08-15T00:00:00Z" })],
        "0.5.14",
      ),
    );

  assert.deepEqual(
    build().map((row) => row.source),
    ["local", "upstream"],
  );
  assert.deepEqual(build(), build());
});

test("an empty or absent upstream list still yields the local history", () => {
  const local = localReleaseRows([
    { version: "0.5.14-2", date: "2026-08-16", bullets: [] },
  ]);
  assert.equal(mergeReleaseTimeline(local, []).length, 1);
  assert.equal(mergeReleaseTimeline(local, undefined).length, 1);
  assert.deepEqual(mergeReleaseTimeline(undefined, undefined), []);
});
