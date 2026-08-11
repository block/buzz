import assert from "node:assert/strict";
import test from "node:test";

import {
  parseDeclaredAsks,
  stripDeclaredAskLines,
  viewerAsks,
} from "./declaredAsks.ts";

test("parses the bold declaration form", () => {
  const asks = parseDeclaredAsks(
    "**Needs Lee, decision:** Ship now or wait for the fix?",
  );
  assert.deepEqual(asks, [
    {
      name: "Lee",
      type: "decision",
      ask: "Ship now or wait for the fix?",
      options: [],
    },
  ]);
});

test("parses the plain declaration form, case-insensitively", () => {
  const asks = parseDeclaredAsks(
    "needs Lee, DECISION: Ship now or wait for the fix?",
  );
  assert.equal(asks.length, 1);
  assert.equal(asks[0].type, "decision");
  assert.equal(asks[0].name, "Lee");
});

test("accepts each of the five declared types", () => {
  for (const type of [
    "decision",
    "approval",
    "question",
    "review",
    "blocked",
  ]) {
    const asks = parseDeclaredAsks(`**Needs Lee, ${type}:** Do the thing.`);
    assert.equal(asks.length, 1, type);
    assert.equal(asks[0].type, type);
  }
});

test("rejects invalid types and malformed lines", () => {
  assert.deepEqual(
    parseDeclaredAsks("**Needs Lee, vibes:** Do the thing."),
    [],
  );
  assert.deepEqual(parseDeclaredAsks("**Needs Lee decision:** no comma."), []);
  assert.deepEqual(parseDeclaredAsks("**Needs Lee, decision:**"), []);
  assert.deepEqual(parseDeclaredAsks("We discussed needs and decisions."), []);
});

test("full display names are captured between Needs and the comma", () => {
  const asks = parseDeclaredAsks(
    "Needs Lee Ntshudisane, review: Review the retention change.",
  );
  assert.equal(asks[0].name, "Lee Ntshudisane");
});

test("adopts 2-4 immediately following bullet options", () => {
  const two = parseDeclaredAsks(
    [
      "**Needs Lee, decision:** Ship now or wait?",
      "- Ship it now.",
      "- Wait for the relay fix.",
    ].join("\n"),
  );
  assert.deepEqual(two[0].options, ["Ship it now.", "Wait for the relay fix."]);

  const four = parseDeclaredAsks(
    [
      "**Needs Lee, decision:** Pick a lane.",
      "- One.",
      "- Two.",
      "- Three.",
      "- Four.",
    ].join("\n"),
  );
  assert.equal(four[0].options.length, 4);
});

test("a blank line detaches the option list", () => {
  const asks = parseDeclaredAsks(
    [
      "**Needs Lee, decision:** Ship now or wait?",
      "",
      "- Ship it now.",
      "- Wait for the relay fix.",
    ].join("\n"),
  );
  assert.deepEqual(asks[0].options, []);
});

test("five or more bullets, or a lone bullet, are not options", () => {
  const five = parseDeclaredAsks(
    [
      "**Needs Lee, decision:** Pick a lane.",
      "- One.",
      "- Two.",
      "- Three.",
      "- Four.",
      "- Five.",
    ].join("\n"),
  );
  assert.deepEqual(five[0].options, []);

  const one = parseDeclaredAsks(
    ["**Needs Lee, decision:** Pick a lane.", "- Only one."].join("\n"),
  );
  assert.deepEqual(one[0].options, []);
});

test("multi-person messages parse all declarations, viewerAsks filters", () => {
  const asks = parseDeclaredAsks(
    [
      "**Needs Lee, review:** Review the retention change.",
      "**Needs Axel, approval:** Approve the deploy window.",
    ].join("\n"),
  );
  assert.equal(asks.length, 2);
  const mine = viewerAsks(asks, "Lee");
  assert.equal(mine.length, 1);
  assert.equal(mine[0].name, "Lee");
  assert.equal(viewerAsks(asks, "Axel").length, 1);
  assert.equal(viewerAsks(asks, "Morgan").length, 0);
  assert.equal(viewerAsks(asks, undefined).length, 0);
});

test("viewer matching accepts full display name or its first word", () => {
  const asks = parseDeclaredAsks(
    "**Needs Lee, question:** Did the backup run?",
  );
  assert.equal(viewerAsks(asks, "Lee Ntshudisane").length, 1);
  assert.equal(viewerAsks(asks, "lee").length, 1);
  const fullName = parseDeclaredAsks(
    "**Needs Lee Ntshudisane, question:** Did the backup run?",
  );
  assert.equal(viewerAsks(fullName, "Lee Ntshudisane").length, 1);
  // Declared full name does not match a different person named Lee X.
  assert.equal(viewerAsks(fullName, "Lee Axelsson").length, 0);
});

test("multiple asks for the same viewer are all returned in order", () => {
  const asks = parseDeclaredAsks(
    [
      "**Needs Lee, question:** Did the overnight backup run?",
      "**Needs Lee, review:** Review the retention change when you can.",
    ].join("\n"),
  );
  assert.equal(asks.length, 2);
  const mine = viewerAsks(asks, "Lee");
  assert.equal(mine.length, 2);
  assert.equal(mine[0].type, "question");
  assert.equal(mine[1].type, "review");
});

test("stripDeclaredAskLines removes declarations and attached bullets only", () => {
  const stripped = stripDeclaredAskLines(
    [
      "Status update for the room.",
      "**Needs Axel, decision:** Ship now or wait?",
      "- Ship it now.",
      "- Wait for the relay fix.",
      "Everything else is on track.",
    ].join("\n"),
  );
  assert.equal(
    stripped,
    ["Status update for the room.", "Everything else is on track."].join("\n"),
  );
});
