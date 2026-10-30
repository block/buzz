import assert from "node:assert/strict";
import { test } from "node:test";

import {
  buildConditionExpression,
  buildConditionExpressions,
  conditionFieldsForTrigger,
  conditionOperatorsForField,
  conditionValueError,
  parseConditionExpression,
  parseConditionExpressions,
} from "./workflowConditionExpression.ts";

const AUTHOR = "a".repeat(64);
const MESSAGE_ID = "b".repeat(64);

test("builds and escapes supported text conditions", () => {
  assert.equal(
    buildConditionExpression({
      field: "trigger_text",
      operator: "contains",
      value: 'deploy "buzz"\\path',
      webhookField: "",
    }),
    'str_contains(trigger_text, "deploy \\"buzz\\"\\\\path")',
  );
});

test("limits opaque identifiers to equality and validates 64-char hex", () => {
  for (const field of [
    "trigger_author",
    "trigger_emoji",
    "trigger_message_id",
    "future_id",
  ]) {
    assert.deepEqual(conditionOperatorsForField(field), [
      "equals",
      "not_equals",
    ]);
  }
  assert.equal(
    conditionValueError("trigger_author", "abc"),
    "Enter a 64-character hex pubkey.",
  );
  assert.equal(conditionValueError("trigger_author", AUTHOR), null);
  assert.equal(
    conditionValueError("trigger_message_id", "z".repeat(64)),
    "Enter a 64-character hex event ID.",
  );
  assert.equal(conditionValueError("trigger_message_id", MESSAGE_ID), null);
});

test("normalizes generated hex identifiers to lowercase", () => {
  const uppercaseAuthor = "A".repeat(64);
  const uppercaseMessageId = "B".repeat(64);
  assert.equal(
    buildConditionExpressions([
      {
        field: "trigger_author",
        operator: "equals",
        value: uppercaseAuthor,
        webhookField: "",
      },
      {
        field: "trigger_message_id",
        operator: "not_equals",
        value: uppercaseMessageId,
        webhookField: "",
      },
    ]),
    `trigger_author == "${AUTHOR}" && trigger_message_id != "${MESSAGE_ID}"`,
  );
});

test("exposes only trigger-relevant local fields", () => {
  assert.deepEqual(
    conditionFieldsForTrigger("reaction_added").map(({ value }) => value),
    ["trigger_emoji", "trigger_author", "trigger_message_id"],
  );
  assert.deepEqual(
    conditionFieldsForTrigger("message_posted").map(({ value }) => value),
    ["trigger_text", "trigger_author"],
  );
  for (const trigger of [
    "message_posted",
    "diff_posted",
    "reaction_added",
    "webhook",
    "schedule",
  ]) {
    assert.equal(
      conditionFieldsForTrigger(trigger).some(
        ({ value }) => value === "trigger_channel_id",
      ),
      false,
    );
  }
});

test("round-trips reaction emoji, author, and message ID conjunctions", () => {
  const conditions = [
    {
      field: "trigger_emoji",
      operator: "equals",
      value: "👾",
      webhookField: "",
    },
    {
      field: "trigger_author",
      operator: "not_equals",
      value: AUTHOR,
      webhookField: "",
    },
    {
      field: "trigger_message_id",
      operator: "equals",
      value: MESSAGE_ID,
      webhookField: "",
    },
  ];
  const expression = buildConditionExpressions(conditions);
  assert.equal(
    expression,
    `trigger_emoji == "👾" && trigger_author != "${AUTHOR}" && trigger_message_id == "${MESSAGE_ID}"`,
  );
  assert.deepEqual(
    parseConditionExpressions(expression, "reaction_added"),
    conditions,
  );
});

test("does not split conjunctions inside values and rejects duplicates", () => {
  const expression = `str_contains(trigger_text, "one && two") && trigger_author == "${AUTHOR}"`;
  assert.equal(
    parseConditionExpressions(expression, "message_posted")?.length,
    2,
  );
  assert.equal(
    parseConditionExpressions(
      `trigger_author == "${AUTHOR}" && trigger_author != "${MESSAGE_ID}"`,
      "message_posted",
    ),
    null,
  );
});

test("keeps unsupported expressions in advanced mode", () => {
  assert.equal(
    parseConditionExpression("trigger_timestamp > 0", "message_posted"),
    null,
  );
  assert.equal(
    parseConditionExpression('trigger_emoji == "👍"', "message_posted"),
    null,
  );
  assert.equal(
    parseConditionExpression('trigger_author == "abc"', "message_posted"),
    null,
  );
  assert.equal(
    parseConditionExpression("str_len(trigger_author) == 0", "message_posted"),
    null,
  );
  assert.equal(
    parseConditionExpression(
      "str_len(trigger_message_id) > 0",
      "reaction_added",
    ),
    null,
  );
});
