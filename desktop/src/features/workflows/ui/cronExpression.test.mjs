import assert from "node:assert/strict";
import test from "node:test";

import {
  CRON_FIELD_DEFINITIONS,
  cronExpressionError,
  cronExpressionFromFields,
  cronFieldsFromPaste,
  validateCronField,
  validateCronFields,
} from "./cronExpression.ts";

test("accepts supported five-field cron syntax", () => {
  for (const expression of [
    "0 9 * * 1-5",
    "*/15 * * * *",
    "0 */2 1,15 JAN,MAR MON-FRI",
  ]) {
    const result = cronFieldsFromPaste(expression);
    assert.equal(result.ok, true);
    assert.deepEqual(validateCronFields(result.fields), [
      null,
      null,
      null,
      null,
      null,
    ]);
    assert.equal(cronExpressionFromFields(result.fields), expression);
    assert.equal(cronExpressionError(expression), null);
  }
});

test("validates cron field ranges and structure locally", () => {
  assert.equal(
    validateCronField("60", CRON_FIELD_DEFINITIONS[0]),
    "Minute must be between 0 and 59.",
  );
  assert.equal(
    validateCronField("5-2", CRON_FIELD_DEFINITIONS[2]),
    "Day range must go from lower to higher.",
  );
  assert.equal(
    validateCronField("*/0", CRON_FIELD_DEFINITIONS[1]),
    "Hour step must be a positive whole number.",
  );
});

test("whole-expression validation requires exactly five fields", () => {
  assert.deepEqual(cronFieldsFromPaste("0 9 * *"), {
    error: "Paste a 5-field cron expression. Found 4 fields.",
    ok: false,
  });
  assert.match(cronExpressionError("not-a-cron"), /Found 1 field/);
});
