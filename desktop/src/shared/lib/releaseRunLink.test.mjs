import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReleaseRunLink,
  parseReleaseRunLink,
  releaseRunViewState,
} from "./releaseRunLink.ts";

const payload = {
  version: 1,
  runId: "deezer-2026-08-22T06:00:33.651Z",
  runName: "Deezer release re-identification",
  status: "completed",
  checked: 40,
  released: 3,
  held: 37,
  sourceHealth: "Deezer healthy",
  finishedAt: "2026-08-22T06:00:33.651Z",
  tracks: [
    {
      id: "track-1",
      artist: "D Stone",
      title: "Total Unison",
      version: "Original Mix",
      label: "Heist Recordings",
      releaseDate: "2026-08-22",
      artworkUrl: "https://cdn.example.com/total-unison.jpg",
      source: "Deezer",
      sourceUrl: "https://www.deezer.com/track/123",
      detailsUrl: "https://team.trakthat.app/releases?track=track-1",
    },
    {
      id: "track-2",
      artist: "Fleur Shore",
      title: "Higher",
      label: "Cuttin' Headz",
      releaseDate: "2026-08-22",
      source: "Deezer",
    },
    {
      id: "track-3",
      artist: "M-High",
      title: "The Answer",
      releaseDate: "2026-08-22",
      source: "Deezer",
    },
  ],
};

test("release run link round-trips a complete bounded payload", () => {
  const href = buildReleaseRunLink(payload);
  assert.match(href, /^buzz:\/\/release-run\?data=[A-Za-z0-9_-]+$/);
  assert.deepEqual(parseReleaseRunLink(href), payload);
  assert.equal(releaseRunViewState(payload), "ready");
});

test("release run parser rejects a mismatched released count", () => {
  const href = buildReleaseRunLink({ ...payload, released: 2 });
  assert.equal(parseReleaseRunLink(href), null);
});

test("release run parser rejects insecure artwork and destinations", () => {
  const href = buildReleaseRunLink({
    ...payload,
    tracks: [
      { ...payload.tracks[0], artworkUrl: "http://cdn.example.com/cover.jpg" },
      ...payload.tracks.slice(1),
    ],
  });
  assert.equal(parseReleaseRunLink(href), null);
});

test("release run parser rejects duplicate and unknown query parameters", () => {
  const href = buildReleaseRunLink(payload);
  const data = new URL(href).searchParams.get("data");
  assert.equal(
    parseReleaseRunLink(`${href}&data=${data}`),
    null,
    "duplicate data parameter",
  );
  assert.equal(parseReleaseRunLink(`${href}&open=external`), null);
});

test("release run view state distinguishes live, empty, and failed runs", () => {
  const empty = { ...payload, released: 0, tracks: [] };
  assert.equal(releaseRunViewState({ ...empty, status: "running" }), "loading");
  assert.equal(releaseRunViewState({ ...empty, status: "completed" }), "empty");
  assert.equal(releaseRunViewState({ ...empty, status: "failed" }), "failed");
});
