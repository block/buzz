import { i18n } from "@/i18n";

export const CRON_FIELD_DEFINITIONS = [
  { id: "minute", max: 59, min: 0 },
  { id: "hour", max: 23, min: 0 },
  { id: "day", max: 31, min: 1 },
  {
    aliases: [
      "JAN",
      "FEB",
      "MAR",
      "APR",
      "MAY",
      "JUN",
      "JUL",
      "AUG",
      "SEP",
      "OCT",
      "NOV",
      "DEC",
    ],
    id: "month",
    max: 12,
    min: 1,
  },
  {
    aliases: ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"],
    id: "weekday",
    max: 7,
    min: 0,
  },
] as const;

export type CronFields = [string, string, string, string, string];

/** Cron field captions resolve at call time so they follow the live language. */
export function cronFieldLabel(id: CronFieldId): string {
  switch (id) {
    case "minute":
      return i18n.t("workflows.cron.field-minute");
    case "hour":
      return i18n.t("workflows.cron.field-hour");
    case "day":
      return i18n.t("workflows.cron.field-day");
    case "month":
      return i18n.t("workflows.cron.field-month");
    case "weekday":
      return i18n.t("workflows.cron.field-weekday");
  }
}

export function cronFieldsFromExpression(expression: string): CronFields {
  const values = expression.trim() ? expression.trim().split(/\s+/) : [];
  return [
    values[0] ?? "",
    values[1] ?? "",
    values[2] ?? "",
    values[3] ?? "",
    values[4] ?? "",
  ];
}

export function cronExpressionFromFields(fields: CronFields): string {
  return fields.join(" ");
}

export function normalizeCronExpression(expression: string): string {
  return expression.trim().replace(/\s+/g, " ");
}

type CronFieldDefinition = (typeof CRON_FIELD_DEFINITIONS)[number];
type CronFieldId = CronFieldDefinition["id"];

export function cronFieldsFromPaste(
  pastedValue: string,
): { fields: CronFields; ok: true } | { error: string; ok: false } {
  const values = pastedValue.trim().split(/\s+/);
  if (values.length !== CRON_FIELD_DEFINITIONS.length) {
    return {
      error: i18n.t("workflows.cron.paste-field-count", {
        count: values.length,
      }),
      ok: false,
    };
  }
  return { fields: values as CronFields, ok: true };
}

function atomError(
  atom: string,
  definition: CronFieldDefinition,
): string | null {
  const upperAtom = atom.toUpperCase();
  if (
    "aliases" in definition &&
    definition.aliases.includes(upperAtom as never)
  ) {
    return null;
  }
  const label = cronFieldLabel(definition.id);
  if (!/^\d+$/.test(atom)) {
    return i18n.t("workflows.cron.error-unsupported-value", {
      field: label,
      value: atom,
    });
  }

  const value = Number(atom);
  if (value < definition.min || value > definition.max) {
    return i18n.t("workflows.cron.error-out-of-range", {
      field: label,
      min: definition.min,
      max: definition.max,
    });
  }
  return null;
}

function segmentError(
  segment: string,
  definition: CronFieldDefinition,
): string | null {
  const label = cronFieldLabel(definition.id);
  const stepParts = segment.split("/");
  if (stepParts.length > 2 || stepParts.some((part) => !part)) {
    return i18n.t("workflows.cron.error-invalid-step", { field: label });
  }

  const [base, step] = stepParts;
  if (step !== undefined) {
    if (!/^\d+$/.test(step) || Number(step) < 1) {
      return i18n.t("workflows.cron.error-step-not-positive-integer", {
        field: label,
      });
    }
  }

  if (base === "*") return null;

  const rangeParts = base.split("-");
  if (rangeParts.length > 2 || rangeParts.some((part) => !part)) {
    return i18n.t("workflows.cron.error-invalid-range", { field: label });
  }

  const startError = atomError(rangeParts[0], definition);
  if (startError) return startError;
  if (rangeParts.length === 1) return null;

  const endError = atomError(rangeParts[1], definition);
  if (endError) return endError;

  const start = Number(rangeParts[0]);
  const end = Number(rangeParts[1]);
  if (Number.isFinite(start) && Number.isFinite(end) && start > end) {
    return i18n.t("workflows.cron.error-range-descending", { field: label });
  }
  return null;
}

export function validateCronField(
  value: string,
  definition: CronFieldDefinition,
): string | null {
  const label = cronFieldLabel(definition.id);
  if (!value) return i18n.t("workflows.cron.error-required", { field: label });

  const segments = value.split(",");
  if (segments.some((segment) => !segment)) {
    return i18n.t("workflows.cron.error-empty-list-item", { field: label });
  }

  for (const segment of segments) {
    const error = segmentError(segment, definition);
    if (error) return error;
  }
  return null;
}

export function validateCronFields(fields: CronFields): Array<string | null> {
  return fields.map((field, index) =>
    validateCronField(field, CRON_FIELD_DEFINITIONS[index]),
  );
}

export function cronExpressionError(expression: string): string | null {
  const parsed = cronFieldsFromPaste(expression);
  if (!parsed.ok) return parsed.error;
  return validateCronFields(parsed.fields).find(Boolean) ?? null;
}
