import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  CRON_FIELD_DEFINITIONS,
  cronExpressionFromFields,
  cronFieldsFromExpression,
  cronFieldsFromPaste,
  normalizeCronExpression,
  validateCronFields,
} from "./cronExpression";
import type { CronFields } from "./cronExpression";

export function CronExpressionInput({
  disabled,
  onChange,
  value,
}: {
  disabled?: boolean;
  onChange: (value: string) => void;
  value: string;
}) {
  const [fields, setFields] = React.useState<CronFields>(() =>
    cronFieldsFromExpression(value),
  );
  const [pasteError, setPasteError] = React.useState<string | null>(null);
  const inputRefs = React.useRef<Array<HTMLInputElement | null>>([]);
  const localValue = React.useRef(normalizeCronExpression(value));
  const validationErrors = validateCronFields(fields);
  const firstError = pasteError ?? validationErrors.find(Boolean) ?? null;
  const messageId = "wf-trigger-cron-message";

  React.useEffect(() => {
    const nextValue = normalizeCronExpression(value);
    if (nextValue !== localValue.current) {
      setFields(cronFieldsFromExpression(value));
      localValue.current = nextValue;
      setPasteError(null);
    }
  }, [value]);

  const commitFields = (nextFields: CronFields) => {
    const expression = cronExpressionFromFields(nextFields);
    setFields(nextFields);
    setPasteError(null);
    localValue.current = normalizeCronExpression(expression);
    onChange(expression);
  };

  const focusField = (index: number) => {
    inputRefs.current[index]?.focus();
    inputRefs.current[index]?.select();
  };

  return (
    <fieldset className="space-y-1.5">
      <legend className="text-xs font-medium text-muted-foreground">
        Cron expression
      </legend>
      <div aria-describedby={messageId} className="space-y-1">
        <div className="grid grid-cols-5 gap-px px-px">
          {CRON_FIELD_DEFINITIONS.map((definition, index) => (
            <label
              className={cn(
                "min-w-0 truncate px-1 text-center text-2xs text-muted-foreground",
                validationErrors[index] && "text-destructive",
              )}
              htmlFor={`wf-trigger-cron-${definition.label.toLowerCase()}`}
              key={definition.label}
            >
              {definition.label}
            </label>
          ))}
        </div>
        <div
          className={cn(
            "grid grid-cols-5 overflow-hidden rounded-lg border border-input/50 bg-background",
            "focus-within:border-ring focus-within:ring-1 focus-within:ring-ring",
            disabled && "opacity-50",
          )}
        >
          {CRON_FIELD_DEFINITIONS.map((definition, index) => (
            <input
              aria-describedby={messageId}
              aria-invalid={Boolean(validationErrors[index])}
              aria-label={definition.label}
              autoCapitalize="characters"
              autoCorrect="off"
              className={cn(
                "h-11 min-w-0 border-l border-input/50 bg-transparent px-1 text-center font-mono text-sm text-foreground outline-hidden first:border-l-0",
                "placeholder:text-muted-foreground/60 focus:z-10 focus:bg-muted/35",
                "disabled:cursor-not-allowed",
                validationErrors[index] &&
                  "bg-destructive/5 text-destructive focus:bg-destructive/10",
              )}
              disabled={disabled}
              id={`wf-trigger-cron-${definition.label.toLowerCase()}`}
              key={definition.label}
              onChange={(event) => {
                const nextFields = [...fields] as CronFields;
                nextFields[index] = event.target.value.replace(/\s/g, "");
                commitFields(nextFields);
              }}
              onKeyDown={(event) => {
                const input = event.currentTarget;
                if (event.key === " " && index < fields.length - 1) {
                  event.preventDefault();
                  focusField(index + 1);
                } else if (
                  event.key === "Backspace" &&
                  !input.value &&
                  index > 0
                ) {
                  event.preventDefault();
                  focusField(index - 1);
                } else if (
                  event.key === "ArrowLeft" &&
                  input.selectionStart === 0 &&
                  index > 0
                ) {
                  event.preventDefault();
                  focusField(index - 1);
                } else if (
                  event.key === "ArrowRight" &&
                  input.selectionStart === input.value.length &&
                  index < fields.length - 1
                ) {
                  event.preventDefault();
                  focusField(index + 1);
                }
              }}
              onPaste={(event) => {
                const pastedValue =
                  event.clipboardData.getData("text/plain") ||
                  event.clipboardData.getData("text");
                if (!/\s/.test(pastedValue.trim())) return;

                event.preventDefault();
                const result = cronFieldsFromPaste(pastedValue);
                if (!result.ok) {
                  setPasteError(result.error);
                  return;
                }
                commitFields(result.fields);
              }}
              placeholder="*"
              ref={(element) => {
                inputRefs.current[index] = element;
              }}
              spellCheck={false}
              value={fields[index]}
            />
          ))}
        </div>
      </div>
      <p
        className={cn(
          "text-xs text-muted-foreground",
          firstError && "text-destructive",
        )}
        id={messageId}
        role={firstError ? "alert" : undefined}
      >
        {firstError ??
          "UTC · Paste all 5 fields, or use wildcards, lists, ranges, and steps."}
      </p>
    </fieldset>
  );
}
