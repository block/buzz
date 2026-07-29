import assert from "node:assert/strict";
import test from "node:test";

import {
  programEventTone,
  strongestProgramEventTone,
  weekAllDayPlacement,
} from "./eventPresentation.ts";

function event(overrides = {}) {
  return {
    allDay: true,
    start: "2026-07-27T00:00:00+10:00",
    end: "2026-07-28T00:00:00+10:00",
    location: null,
    ...overrides,
  };
}

test("all-day ship locations classify as sea, port, or neutral", () => {
  assert.equal(
    programEventTone(event({ allDay: true, location: "Sea" })),
    "sea",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "At Sea" })),
    "sea",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "SEA – Coral Sea" })),
    "sea",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "FBE" })),
    "port",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "FBW" })),
    "port",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "Sydney" })),
    "port",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "Fremantle" })),
    "port",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "Seaside" })),
    "port",
  );
  assert.equal(
    programEventTone(event({ allDay: true, location: "  " })),
    "neutral",
  );
  assert.equal(
    programEventTone(event({ allDay: false, location: "Sea" })),
    "neutral",
  );
});

test("a cross-week all-day event is clipped to its visible columns", () => {
  assert.deepEqual(
    weekAllDayPlacement(
      event({
        allDay: true,
        start: "2026-07-29T00:00:00+10:00",
        end: "2026-08-05T00:00:00+10:00",
      }),
      {
        start: "2026-07-27T00:00:00+10:00",
        end: "2026-08-03T00:00:00+10:00",
      },
      "Australia/Sydney",
    ),
    { startColumn: 3, span: 5 },
  );
});

test("Week placement rejects timed and non-overlapping events", () => {
  const range = {
    start: "2026-07-27T00:00:00+10:00",
    end: "2026-08-03T00:00:00+10:00",
  };
  assert.equal(
    weekAllDayPlacement(
      event({
        allDay: false,
        start: "2026-07-29T08:00:00+10:00",
        end: "2026-07-29T09:00:00+10:00",
      }),
      range,
      "Australia/Sydney",
    ),
    null,
  );
  assert.equal(
    weekAllDayPlacement(
      event({
        start: "2026-08-03T00:00:00+10:00",
        end: "2026-08-04T00:00:00+10:00",
      }),
      range,
      "Australia/Sydney",
    ),
    null,
  );
});

test("a calendar cell preserves the most operationally significant program tone", () => {
  assert.equal(
    strongestProgramEventTone([
      event({ allDay: false, location: "Sea" }),
      event({ allDay: true, location: "FBE" }),
    ]),
    "port",
  );
  assert.equal(
    strongestProgramEventTone([
      event({ allDay: true, location: "FBE" }),
      event({ allDay: true, location: "Sea" }),
    ]),
    "sea",
  );
  assert.equal(strongestProgramEventTone([]), "neutral");
});
