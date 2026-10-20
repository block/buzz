import assert from "node:assert/strict";
import test from "node:test";

import {
  MESSAGE_TEXT_CONDITION_OPERATORS,
  buildMessageTextCondition,
  parseMessageTextCondition,
} from "./workflowMessageTextCondition.ts";

test("builds and parses every structured message-text condition", () => {
  for (const operator of MESSAGE_TEXT_CONDITION_OPERATORS) {
    const condition = { operator, value: "deploy" };
    const expression = buildMessageTextCondition(condition);
    assert.notEqual(expression, null);
    assert.deepEqual(parseMessageTextCondition(expression), {
      operator,
      value:
        operator === "is_not_empty" || operator === "is_empty" ? "" : "deploy",
    });
  }
});

test("escapes and restores evalexpr quote and backslash literals", () => {
  const condition = {
    operator: "equals",
    value: 'say "hello"\\world',
  };
  const expression = buildMessageTextCondition(condition);

  assert.equal(expression, 'trigger_text == "say \\"hello\\"\\\\world"');
  assert.deepEqual(parseMessageTextCondition(expression), condition);
});

test("parses only exact expressions emitted by the structured form", () => {
  for (const expression of [
    ' str_contains(trigger_text, "deploy")',
    'str_contains(trigger_text,"deploy")',
    'str_contains(other_text, "deploy")',
    'str_contains(trigger_text, "one") && trigger_author == "abc"',
    'trigger_text == "bad\\nvalue"',
  ]) {
    assert.equal(parseMessageTextCondition(expression), null, expression);
  }
});

test("does not build value operators until text is present", () => {
  assert.equal(
    buildMessageTextCondition({ operator: "contains", value: "   " }),
    null,
  );
  assert.equal(
    buildMessageTextCondition({ operator: "is_empty", value: "ignored" }),
    "str_len(trigger_text) == 0",
  );
});
