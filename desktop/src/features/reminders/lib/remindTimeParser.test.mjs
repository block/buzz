import assert from "node:assert/strict";
import test from "node:test";

import {
  createRemindZone,
  formatRemindDue,
  parseRemindCommand,
  REMIND_PARSE_ERROR_MESSAGES,
} from "./remindTimeParser.ts";

// The vectors in docs/slash-commands.md are in America/Chicago, where DST ends
// 2026-11-01. The zone is injected so they hold under any process TZ.
const zone = createRemindZone("America/Chicago");

function wall(local) {
  const [y, mo, d, h, mi] = local.match(/\d+/g).map(Number);
  return zone.wallToInstant(y, mo, d, h, mi);
}

function local(instant) {
  const w = zone.instantToWall(instant);
  const two = (n) => String(n).padStart(2, "0");
  return `${w.year}-${two(w.month)}-${two(w.day)} ${two(w.hour)}:${two(w.minute)}`;
}

const NOW = wall("2026-10-28 10:00");

function parse(args) {
  return parseRemindCommand(args, { now: NOW, zone });
}

test("the injected zone matches the spec calendar", () => {
  assert.equal(local(NOW), "2026-10-28 10:00");
  assert.equal(zone.instantToWall(NOW).weekday, 3, "Wednesday");
  assert.equal(
    wall("2026-11-04 10:00") - NOW,
    (7 * 24 + 1) * 3_600_000,
    "DST ends between the two dates, so a week is 169 hours",
  );
  assert.equal(new Date(NOW).toISOString(), "2026-10-28T15:00:00.000Z");
  assert.equal(
    new Date(wall("2026-11-04 10:00")).toISOString(),
    "2026-11-04T16:00:00.000Z",
  );
});

test("createRemindZone rejects an unknown zone", () => {
  assert.throws(() => createRemindZone("Not/AZone"), RangeError);
});

const WHEN_VECTORS = [
  ["in 20 minutes", "2026-10-28 10:20"],
  ["in an hour", "2026-10-28 11:00"],
  ["in 2 hrs", "2026-10-28 12:00"],
  ["in 3 days", "2026-10-31 10:00"],
  ["in 1 week", "2026-11-04 10:00"],
  ["at 3pm", "2026-10-28 15:00"],
  ["at 3:30 pm", "2026-10-28 15:30"],
  ["at 15:00", "2026-10-28 15:00"],
  ["at 9am", "2026-10-29 09:00"],
  ["at 10am", "2026-10-29 10:00"],
  ["at noon", "2026-10-28 12:00"],
  ["at midnight", "2026-10-29 00:00"],
  ["tomorrow", "2026-10-29 09:00"],
  ["tomorrow at 4pm", "2026-10-29 16:00"],
  ["on friday", "2026-10-30 09:00"],
  ["Friday at 2pm", "2026-10-30 14:00"],
  ["on wednesday", "2026-11-04 09:00"],
  ["next monday", "2026-11-02 09:00"],
  ["next week", "2026-11-02 09:00"],
  ["at 3", "ambiguousHour"],
  ["at 25:00", "invalidTime"],
  ["at 13pm", "invalidTime"],
  ["in 0 minutes", "invalidTime"],
  ["in 400 days", "outOfRange"],
  ["yesterday", "missingTime"],
];

const COMMAND_VECTORS = [
  [
    "me to submit the report in 20 minutes",
    { note: "submit the report", due: "2026-10-28 10:20" },
  ],
  [
    "me tomorrow at 4pm to call Sam",
    { note: "call Sam", due: "2026-10-29 16:00" },
  ],
  [
    "me check the deploy at 3pm",
    { note: "check the deploy", due: "2026-10-28 15:00" },
  ],
  ["me in 2 hours", { error: "missingNote" }],
  ["me to stretch", { error: "missingTime" }],
  ["@sam to stretch in 1 hour", { error: "notPersonal" }],
  ["", { error: "usage" }],
  ["me", { error: "usage" }],
  ["mega stretch in 1 hour", { error: "notPersonal" }],
  ["ME to Stretch IN 5 MIN", { note: "Stretch", due: "2026-10-28 10:05" }],
  ["me tomorrow at 4pm", { error: "missingNote" }],
  [
    "me to ring at 3pm tomorrow at 4pm",
    { note: "ring at 3pm", due: "2026-10-29 16:00" },
  ],
  [
    "me in an hour to call Sam on friday",
    { note: "in an hour to call Sam", due: "2026-10-30 09:00" },
  ],
  ["me to toggle at 3", { error: "ambiguousHour" }],
];

function assertOutcome(result, expected) {
  if (expected.error) {
    assert.deepEqual(result, { ok: false, error: expected.error });
  } else {
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.equal(result.note, expected.note);
    assert.equal(local(result.dueAt), expected.due);
  }
}

for (const [when, expected] of WHEN_VECTORS) {
  test(`<when> "${when}"`, () => {
    assertOutcome(
      parse(`me stretch ${when}`),
      expected.includes(" ")
        ? { note: "stretch", due: expected }
        : { error: expected },
    );
  });
}

for (const [args, expected] of COMMAND_VECTORS) {
  test(`/remind ${args}`, () => {
    assertOutcome(parse(args), expected);
  });
}

test("a leading <when> works with the same phrases as a trailing one", () => {
  for (const [when, expected] of WHEN_VECTORS) {
    if (!expected.includes(" ")) continue;
    const result = parse(`me ${when} to stretch`);
    assert.equal(result.ok, true, when);
    assert.equal(result.note, "stretch", when);
    assert.equal(local(result.dueAt), expected, when);
  }
});

test("only one leading 'to ' is stripped from the note", () => {
  const result = parse("me to to stretch in 5 minutes");
  assert.equal(result.note, "to stretch");
});

test("a time that is not after now, or over 365 days away, is out of range", () => {
  const at = (args, now) => parseRemindCommand(args, { now, zone });
  assert.deepEqual(at("me x at 10:00", wall("2026-10-28 10:00")), {
    ok: true,
    note: "x",
    dueAt: wall("2026-10-29 10:00"),
  });
  assert.deepEqual(at("me x in 365 days", NOW), {
    ok: true,
    note: "x",
    dueAt: wall("2027-10-28 10:00"),
  });
  assert.deepEqual(at("me x in 52 weeks", NOW), {
    ok: true,
    note: "x",
    dueAt: wall("2027-10-27 10:00"),
  });
  assert.deepEqual(at("me x in 366 days", NOW), {
    ok: false,
    error: "outOfRange",
  });
});

test("minutes and hours are exact durations; days keep the local time across DST", () => {
  const eve = wall("2026-10-31 12:00");
  const hours = parseRemindCommand("me x in 24 hours", { now: eve, zone });
  const days = parseRemindCommand("me x in 1 day", { now: eve, zone });
  assert.equal(hours.dueAt - eve, 24 * 3_600_000);
  assert.equal(local(hours.dueAt), "2026-11-01 11:00");
  assert.equal(local(days.dueAt), "2026-11-01 12:00");
  assert.equal(days.dueAt - eve, 25 * 3_600_000);
});

test("the injected zone, not the process zone, decides wall-clock results", () => {
  const utc = createRemindZone("UTC");
  const now = Date.UTC(2026, 9, 28, 15, 0);
  const inUtc = parseRemindCommand("me x tomorrow", { now, zone: utc });
  const inChicago = parseRemindCommand("me x tomorrow", { now, zone });
  assert.equal(new Date(inUtc.dueAt).toISOString(), "2026-10-29T09:00:00.000Z");
  assert.equal(
    new Date(inChicago.dueAt).toISOString(),
    "2026-10-29T14:00:00.000Z",
  );
});

test("rejections carry the same copy as mobile", () => {
  assert.deepEqual(REMIND_PARSE_ERROR_MESSAGES, {
    usage: null,
    notPersonal: "Reminders are personal. Use /remind me …",
    missingNote: "Say what to be reminded about.",
    missingTime:
      "Say when, such as “in 20 minutes”, “at 3pm” or “tomorrow at 4pm”.",
    ambiguousHour: "Add am or pm to the hour, such as “at 3pm”.",
    invalidTime: "That is not a valid time.",
    outOfRange: "Pick a time within the next 365 days.",
  });
});

test("formatRemindDue reads like the mobile toast", () => {
  const due = new Date(2026, 9, 30, 14, 0).getTime();
  assert.equal(formatRemindDue(due), "Fri, Oct 30 at 2:00 PM");
});
