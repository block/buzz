import { i18n } from "@/i18n";
import type { TriggerType } from "./workflowFormTypes";

export const CONDITION_OPERATORS = [
  "contains",
  "not_contains",
  "starts_with",
  "ends_with",
  "equals",
  "not_equals",
  "is_not_empty",
  "is_empty",
] as const;
export type ConditionOperator = (typeof CONDITION_OPERATORS)[number];

const EXACT_MATCH_OPERATORS = [
  "equals",
  "not_equals",
] as const satisfies readonly ConditionOperator[];
const HEX_ID_PATTERN = /^[0-9a-fA-F]{64}$/;

const AUTHOR_FIELD = "trigger_author";
const FIELD_VALUES_BY_TRIGGER: Record<TriggerType, string[]> = {
  message_posted: ["trigger_text", AUTHOR_FIELD],
  diff_posted: ["trigger_text", AUTHOR_FIELD],
  reaction_added: ["trigger_emoji", AUTHOR_FIELD, "trigger_message_id"],
  webhook: [],
  schedule: [],
};

export type ConditionField = { label: string; value: string };
export type ParsedConditionExpression = {
  field: string;
  operator: ConditionOperator;
  value: string;
  webhookField: string;
};

/**
 * Condition field captions resolve at call time so they follow the live
 * language. `trigger_text` is the same evalexpr variable for both the message
 * and diff triggers, so the trigger decides which phrase reads naturally.
 */
export function conditionFieldLabel(
  triggerType: TriggerType,
  field: string,
): string {
  if (field === AUTHOR_FIELD) {
    return i18n.t("workflows.condition.field-author");
  }
  if (field === "trigger_emoji") {
    return i18n.t("workflows.condition.field-reaction-emoji");
  }
  if (field === "trigger_message_id") {
    return i18n.t("workflows.condition.field-message");
  }
  if (field === "trigger_text") {
    return triggerType === "diff_posted"
      ? i18n.t("workflows.condition.field-diff-text")
      : i18n.t("workflows.condition.field-message-text");
  }
  return field;
}

export function conditionFieldsForTrigger(
  triggerType: TriggerType,
): ConditionField[] {
  return FIELD_VALUES_BY_TRIGGER[triggerType].map((value) => ({
    label: conditionFieldLabel(triggerType, value),
    value,
  }));
}

export function conditionOperatorsForField(
  field: string,
): readonly ConditionOperator[] {
  return field === "trigger_author" ||
    field === "trigger_emoji" ||
    field.endsWith("_id")
    ? EXACT_MATCH_OPERATORS
    : CONDITION_OPERATORS;
}

export function defaultConditionOperatorForField(
  field: string,
): ConditionOperator {
  return conditionOperatorsForField(field)[0];
}

export function conditionOperatorNeedsValue(
  operator: ConditionOperator,
): boolean {
  return operator !== "is_not_empty" && operator !== "is_empty";
}

export function conditionValueError(
  field: string,
  value: string,
): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (field === "trigger_author" && !HEX_ID_PATTERN.test(trimmed)) {
    return i18n.t("workflows.condition.error-hex-pubkey");
  }
  if (field.endsWith("_id") && !HEX_ID_PATTERN.test(trimmed)) {
    return i18n.t("workflows.condition.error-hex-event-id");
  }
  return null;
}

function escapeEvalexprString(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

export function buildConditionExpression(
  condition: ParsedConditionExpression,
): string | null {
  const { field, operator } = condition;
  if (operator === "is_not_empty") return `str_len(${field}) > 0`;
  if (operator === "is_empty") return `str_len(${field}) == 0`;
  const value = condition.value.trim();
  if (!value || conditionValueError(field, value)) return null;
  const normalizedValue =
    field === "trigger_author" || field.endsWith("_id")
      ? value.toLowerCase()
      : value;
  const quoted = `"${escapeEvalexprString(normalizedValue)}"`;
  switch (operator) {
    case "contains":
      return `str_contains(${field}, ${quoted})`;
    case "not_contains":
      return `!str_contains(${field}, ${quoted})`;
    case "starts_with":
      return `str_starts_with(${field}, ${quoted})`;
    case "ends_with":
      return `str_ends_with(${field}, ${quoted})`;
    case "equals":
      return `${field} == ${quoted}`;
    case "not_equals":
      return `${field} != ${quoted}`;
    default:
      return null;
  }
}

export function buildConditionExpressions(
  conditions: ParsedConditionExpression[],
): string {
  return conditions
    .map(buildConditionExpression)
    .filter((value): value is string => value !== null)
    .join(" && ");
}

function unescapeEvalexprString(value: string): string {
  return value.replaceAll(/\\(["\\])/g, "$1");
}

function parseField(variable: string, triggerType: TriggerType): string | null {
  return conditionFieldsForTrigger(triggerType).some(
    (field) => field.value === variable,
  )
    ? variable
    : null;
}

export function parseConditionExpression(
  expression: string,
  triggerType: TriggerType,
): ParsedConditionExpression | null {
  const trimmed = expression.trim();
  const empty = /^str_len\(([A-Za-z_][A-Za-z0-9_]*)\) (>|==) 0$/.exec(trimmed);
  if (empty && parseField(empty[1], triggerType)) {
    const operator: ConditionOperator =
      empty[2] === ">" ? "is_not_empty" : "is_empty";
    if (!conditionOperatorsForField(empty[1]).includes(operator)) return null;
    return {
      field: empty[1],
      operator,
      value: "",
      webhookField: "",
    };
  }
  const literal = '"((?:\\\\["\\\\]|[^"\\\\])*)"';
  const fn = new RegExp(
    `^(!)?str_(contains|starts_with|ends_with)\\(([A-Za-z_][A-Za-z0-9_]*), ${literal}\\)$`,
  ).exec(trimmed);
  if (fn && parseField(fn[3], triggerType)) {
    const operator: ConditionOperator = fn[1]
      ? "not_contains"
      : fn[2] === "starts_with"
        ? "starts_with"
        : fn[2] === "ends_with"
          ? "ends_with"
          : "contains";
    const result = {
      field: fn[3],
      operator,
      value: unescapeEvalexprString(fn[4]),
      webhookField: "",
    };
    return conditionOperatorsForField(result.field).includes(result.operator)
      ? result
      : null;
  }
  const equality = new RegExp(
    `^([A-Za-z_][A-Za-z0-9_]*) (!=|==) ${literal}$`,
  ).exec(trimmed);
  if (equality && parseField(equality[1], triggerType)) {
    const result = {
      field: equality[1],
      operator:
        equality[2] === "==" ? ("equals" as const) : ("not_equals" as const),
      value: unescapeEvalexprString(equality[3]),
      webhookField: "",
    };
    return conditionValueError(result.field, result.value) ? null : result;
  }
  return null;
}

function splitTopLevel(
  expression: string,
  operator: "&&" | "||",
): string[] | null {
  const parts: string[] = [];
  let start = 0,
    depth = 0,
    inString = false,
    escaped = false;
  for (let index = 0; index < expression.length; index += 1) {
    const character = expression[index];
    if (inString) {
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === '"') inString = false;
      continue;
    }
    if (character === '"') inString = true;
    else if (character === "(") depth += 1;
    else if (character === ")") {
      depth -= 1;
      if (depth < 0) return null;
    } else if (
      character === operator[0] &&
      expression[index + 1] === operator[1] &&
      depth === 0
    ) {
      const part = expression.slice(start, index).trim();
      if (!part) return null;
      parts.push(part);
      index += 1;
      start = index + 1;
    }
  }
  if (inString || depth !== 0) return null;
  const last = expression.slice(start).trim();
  if (!last) return null;
  parts.push(last);
  return parts;
}

function splitTopLevelConjunctions(expression: string): string[] | null {
  return splitTopLevel(expression, "&&");
}

/**
 * Reports whether `expression` needs parentheses before it can be combined
 * with `&&`. Conjunction binds tighter than disjunction in evalexpr, so a
 * top-level `||` would otherwise absorb the added conjunct. Structurally
 * unparseable expressions are grouped defensively.
 */
export function needsConjunctionGrouping(expression: string): boolean {
  const parts = splitTopLevel(expression, "||");
  return parts === null || parts.length > 1;
}

export function parseConditionExpressions(
  expression: string,
  triggerType: TriggerType,
): ParsedConditionExpression[] | null {
  if (!expression.trim()) return [];
  const parts = splitTopLevelConjunctions(expression);
  if (!parts) return null;
  const parsed = parts.map((part) =>
    parseConditionExpression(part, triggerType),
  );
  if (parsed.some((condition) => condition === null)) return null;
  const conditions = parsed.filter(
    (condition): condition is ParsedConditionExpression => condition !== null,
  );
  return new Set(conditions.map((condition) => condition.field)).size ===
    conditions.length
    ? conditions
    : null;
}
