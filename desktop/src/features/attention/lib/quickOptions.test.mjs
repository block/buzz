import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultDeclaredOptions,
  deriveQuickOptions,
  MORE_DETAIL_OPTION,
} from "./quickOptions.ts";

const BACKUPS_ASK =
  "Does the Backups list show any backup dated today, or is the newest one still the 30 July entry?";

test("approval gets its pair, review gets its pair (locked matrix)", () => {
  assert.deepEqual(
    deriveQuickOptions("approval", "Approve the staging deploy.", ""),
    ["Approve", "Reject", MORE_DETAIL_OPTION],
  );
  // The matrix gives Review its own primary pair, not the approval one.
  assert.deepEqual(
    deriveQuickOptions("review", "Approve the staging deploy.", ""),
    ["Looks good", "Changes needed", MORE_DETAIL_OPTION],
  );
});

test("polar questions get yes/no", () => {
  assert.deepEqual(
    deriveQuickOptions("question", "Is the backup scheduled?", ""),
    ["Yes", "No", MORE_DETAIL_OPTION],
  );
  assert.deepEqual(
    deriveQuickOptions("question", "did the migration finish?", ""),
    ["Yes", "No", MORE_DETAIL_OPTION],
  );
  // Word boundary: "Done" is not "Do".
  assert.deepEqual(
    deriveQuickOptions("question", "Done with the review yet?", ""),
    [],
  );
  // Yes/no is scoped to derived questions, not other ask types.
  assert.deepEqual(
    deriveQuickOptions("review", "Could you look at the plan?", ""),
    ["Looks good", "Changes needed", MORE_DETAIL_OPTION],
  );
  // Polar questions must end with "?".
  assert.deepEqual(
    deriveQuickOptions("question", "Can you ship this today.", ""),
    [],
  );
});

test("regression: a which-of-two question never yields yes/no", () => {
  const options = deriveQuickOptions("question", BACKUPS_ASK, "");
  assert.notDeepEqual(options.slice(0, 2), ["Yes", "No"]);
  assert.deepEqual(options, [
    "the Backups list show any backup dated today",
    "is the newest one still the 30 July entry",
    MORE_DETAIL_OPTION,
  ]);
});

test("A-or-B asks become their two alternatives", () => {
  assert.deepEqual(
    deriveQuickOptions(
      "decision",
      "Should we ship Tuesday or wait for QA?",
      "",
    ),
    ["ship Tuesday", "wait for QA", MORE_DETAIL_OPTION],
  );
});

test("A-or-B outranks yes/no and approval", () => {
  assert.deepEqual(
    deriveQuickOptions("question", "Do we ship now or hold the release?", ""),
    ["ship now", "hold the release", MORE_DETAIL_OPTION],
  );
  assert.deepEqual(
    deriveQuickOptions("approval", "Approve the deploy or roll it back?", ""),
    ["Approve the deploy", "roll it back", MORE_DETAIL_OPTION],
  );
});

test("A-or-B requires exactly one or with two short sides", () => {
  const longSide =
    "keep the current onboarding flow exactly as designed in the last review cycle";
  assert.deepEqual(
    deriveQuickOptions("question", `Should we ship now or ${longSide}?`, ""),
    [],
  );
  assert.deepEqual(
    deriveQuickOptions("question", "Tea or coffee or juice?", ""),
    [],
  );
});

test("regression: numbered lists are context, not answers", () => {
  const content = [
    "Decisions pending:",
    "1. Ship now",
    "2. Wait for QA",
    "3. Cancel the launch",
  ].join("\n");
  assert.deepEqual(deriveQuickOptions("headsUp", null, content), []);
  assert.deepEqual(
    deriveQuickOptions("question", "Which option do we take?", content),
    [],
  );
});

test("no rule matching yields no options", () => {
  assert.deepEqual(
    deriveQuickOptions(
      "decision",
      "Please weigh in on the plan.",
      "Please weigh in on the plan.",
    ),
    [],
  );
  assert.deepEqual(deriveQuickOptions("headsUp", null, "hello"), []);
  // Blocked is deliberately optionless here: Done is its primary action.
  assert.deepEqual(
    deriveQuickOptions("blocked", "I need the staging password.", ""),
    [],
  );
});

test("every derived option set ends with the way out", () => {
  for (const [type, ask] of [
    ["approval", "Approve the deploy."],
    ["review", "Look at the plan."],
    ["question", "Is the backup scheduled?"],
    ["decision", "Ship now or wait for QA?"],
  ]) {
    const options = deriveQuickOptions(type, ask, "");
    assert.equal(
      options[options.length - 1],
      MORE_DETAIL_OPTION,
      `${type} way out`,
    );
  }
});

test("declared defaults cover approval, review and blocked only", () => {
  assert.deepEqual(defaultDeclaredOptions("approval"), [
    "Approve",
    "Reject",
    MORE_DETAIL_OPTION,
  ]);
  assert.deepEqual(defaultDeclaredOptions("review"), [
    "Looks good",
    "Changes needed",
    MORE_DETAIL_OPTION,
  ]);
  assert.deepEqual(defaultDeclaredOptions("blocked"), [
    "I have done it",
    "I cannot do it",
    MORE_DETAIL_OPTION,
  ]);
  assert.deepEqual(defaultDeclaredOptions("decision"), []);
  assert.deepEqual(defaultDeclaredOptions("question"), []);
  assert.deepEqual(defaultDeclaredOptions("headsUp"), []);
});
