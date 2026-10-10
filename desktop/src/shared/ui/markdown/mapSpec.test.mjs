import assert from "node:assert/strict";
import test from "node:test";

import { mapSpecPoints, parseMapSpec } from "./mapSpec.ts";

test("parses markers, lines, and viewport", () => {
  const result = parseMapSpec(
    JSON.stringify({
      center: [-122.42, 37.77],
      zoom: 30,
      height: 5000,
      markers: [{ lng: -122.4, lat: 37.78, label: "HQ", color: "#f00" }],
      lines: [
        {
          coordinates: [
            [-122.4, 37.78],
            [-122.41, 37.76],
          ],
        },
      ],
    }),
  );
  assert.equal(result.ok, true);
  assert.deepEqual(result.spec, {
    center: [-122.42, 37.77],
    zoom: 22,
    height: 800,
    markers: [{ lng: -122.4, lat: 37.78, label: "HQ", color: "#f00" }],
    lines: [
      {
        coordinates: [
          [-122.4, 37.78],
          [-122.41, 37.76],
        ],
        label: undefined,
        color: undefined,
      },
    ],
  });
  assert.deepEqual(mapSpecPoints(result.spec), [
    [-122.4, 37.78],
    [-122.4, 37.78],
    [-122.41, 37.76],
  ]);
});

test("markers alone are enough to frame the map", () => {
  const result = parseMapSpec('{"markers":[{"lng":1,"lat":2}]}');
  assert.equal(result.ok, true);
  assert.equal(result.spec.center, undefined);
});

test("rejects malformed specs with a readable error", () => {
  assert.deepEqual(parseMapSpec("{nope"), {
    ok: false,
    error: "Map block is not valid JSON.",
  });
  assert.equal(parseMapSpec("[]").ok, false);
  assert.equal(parseMapSpec("{}").ok, false);
  assert.equal(parseMapSpec('{"center":[200,0]}').ok, false);
  assert.equal(
    parseMapSpec('{"markers":[{"lng":"x","lat":1}]}').error,
    "markers[0] needs valid lng/lat.",
  );
  assert.equal(
    parseMapSpec('{"lines":[{"coordinates":[[0,0]]}]}').error,
    "lines[0] needs at least two [lng, lat] coordinates.",
  );
});
