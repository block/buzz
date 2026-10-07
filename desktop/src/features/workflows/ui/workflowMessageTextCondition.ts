export const MESSAGE_TEXT_CONDITION_OPERATORS = [
  "contains",
  "not_contains",
  "starts_with",
  "ends_with",
  "equals",
  "not_equals",
  "is_not_empty",
  "is_empty",
] as const;

export type MessageTextConditionOperator =
  (typeof MESSAGE_TEXT_CONDITION_OPERATORS)[number];

export type MessageTextCondition = {
  operator: MessageTextConditionOperator;
  value: string;
};

export const MESSAGE_TEXT_CONDITION_LABELS: Record<
  MessageTextConditionOperator,
  string
> = {
  contains: "Contains",
  not_contains: "Does not contain",
  starts_with: "Starts with",
  ends_with: "Ends with",
  equals: "Equals",
  not_equals: "Does not equal",
  is_not_empty: "Has text",
  is_empty: "Has no text",
};

export function messageTextConditionNeedsValue(
  operator: MessageTextConditionOperator,
): boolean {
  return operator !== "is_not_empty" && operator !== "is_empty";
}

function escapeEvalexprString(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

export function buildMessageTextCondition(
  condition: MessageTextCondition,
): string | null {
  if (condition.operator === "is_not_empty") {
    return "str_len(trigger_text) > 0";
  }
  if (condition.operator === "is_empty") {
    return "str_len(trigger_text) == 0";
  }
  if (!condition.value.trim()) return null;

  const quotedValue = `"${escapeEvalexprString(condition.value)}"`;
  switch (condition.operator) {
    case "contains":
      return `str_contains(trigger_text, ${quotedValue})`;
    case "not_contains":
      return `!str_contains(trigger_text, ${quotedValue})`;
    case "starts_with":
      return `str_starts_with(trigger_text, ${quotedValue})`;
    case "ends_with":
      return `str_ends_with(trigger_text, ${quotedValue})`;
    case "equals":
      return `trigger_text == ${quotedValue}`;
    case "not_equals":
      return `trigger_text != ${quotedValue}`;
  }
}

function unescapeEvalexprString(value: string): string {
  return value.replaceAll(/\\(["\\])/g, "$1");
}

/** Parse only the exact expression shapes emitted by the structured editor. */
export function parseMessageTextCondition(
  expression: string,
): MessageTextCondition | null {
  let parsed: MessageTextCondition | null = null;
  if (expression === "str_len(trigger_text) > 0") {
    parsed = { operator: "is_not_empty", value: "" };
  } else if (expression === "str_len(trigger_text) == 0") {
    parsed = { operator: "is_empty", value: "" };
  } else {
    const stringLiteral = '"((?:\\\\["\\\\]|[^"\\\\])*)"';
    const functionMatch = new RegExp(
      `^(!)?str_(contains|starts_with|ends_with)\\(trigger_text, ${stringLiteral}\\)$`,
    ).exec(expression);
    if (functionMatch) {
      parsed = {
        operator: functionMatch[1]
          ? "not_contains"
          : functionMatch[2] === "starts_with"
            ? "starts_with"
            : functionMatch[2] === "ends_with"
              ? "ends_with"
              : "contains",
        value: unescapeEvalexprString(functionMatch[3]),
      };
    } else {
      const equalityMatch = new RegExp(
        `^trigger_text (!=|==) ${stringLiteral}$`,
      ).exec(expression);
      if (equalityMatch) {
        parsed = {
          operator: equalityMatch[1] === "==" ? "equals" : "not_equals",
          value: unescapeEvalexprString(equalityMatch[2]),
        };
      }
    }
  }

  return parsed && buildMessageTextCondition(parsed) === expression
    ? parsed
    : null;
}
