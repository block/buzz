import { ChevronRight } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { WorkflowAuthorPicker } from "./WorkflowAuthorPicker";
import { WorkflowEmojiField } from "./WorkflowEmojiField";
import { WorkflowMessagePicker } from "./WorkflowMessagePicker";
import { useWorkflowTriggerPresentation } from "./useWorkflowTriggerPresentation";
import { FieldLabel } from "./workflowFormPrimitives";
import {
  buildConditionExpressions,
  conditionFieldsForTrigger,
  conditionOperatorNeedsValue,
  conditionOperatorsForField,
  conditionValueError,
  parseConditionExpressions,
  type ConditionOperator,
  type ParsedConditionExpression,
} from "./workflowConditionExpression";
import type { TriggerType } from "./workflowFormTypes";

const OPERATOR_LABELS: Record<ConditionOperator, string> = {
  contains: "contains",
  not_contains: "does not contain",
  starts_with: "starts with",
  ends_with: "ends with",
  equals: "is",
  not_equals: "is not",
  is_not_empty: "is not empty",
  is_empty: "is empty",
};

function emptyCondition(field: string): ParsedConditionExpression {
  return {
    field,
    operator: conditionOperatorsForField(field)[0],
    value: "",
    webhookField: "",
  };
}

function compact(value: string): string {
  const trimmed = value.trim();
  return trimmed.length <= 20
    ? trimmed
    : `${trimmed.slice(0, 11)}…${trimmed.slice(-6)}`;
}

function fieldUsesFullHeightPicker(field: string): boolean {
  return field === "trigger_author" || field === "trigger_message_id";
}

function fieldPlaceholder(field: string): string {
  if (field === "trigger_author") return "64-character hex pubkey";
  if (field === "trigger_message_id") return "64-character hex event ID";
  return "e.g. deploy";
}

export function WorkflowTriggerConditions({
  conditionDrafts,
  disabled,
  onChange,
  onConditionDraftsChange,
  triggerType,
  value,
  workflowChannelId,
}: {
  conditionDrafts: ParsedConditionExpression[] | null;
  disabled?: boolean;
  onChange: (value: string) => void;
  onConditionDraftsChange: (drafts: ParsedConditionExpression[] | null) => void;
  triggerType: Extract<
    TriggerType,
    "message_posted" | "diff_posted" | "reaction_added"
  >;
  value: string;
  workflowChannelId?: string | null;
}) {
  const parsedValue = React.useMemo(
    () => parseConditionExpressions(value, triggerType),
    [triggerType, value],
  );
  const [mode, setMode] = React.useState<"basic" | "advanced">(() =>
    value && parsedValue === null ? "advanced" : "basic",
  );
  const [expandedField, setExpandedField] = React.useState<string | null>(
    () => conditionFieldsForTrigger(triggerType)[0]?.value ?? null,
  );
  const conditions = conditionDrafts ?? parsedValue ?? [];
  const localValue = React.useRef(value);
  const triggerPresentation = useWorkflowTriggerPresentation(
    {
      filter: conditions.length
        ? buildConditionExpressions(conditions)
        : undefined,
      on: triggerType,
    },
    workflowChannelId,
  );

  React.useEffect(() => {
    if (value === localValue.current) return;
    localValue.current = value;
    const parsed = parseConditionExpressions(value, triggerType);
    if (parsed) {
      onConditionDraftsChange(null);
      setMode("basic");
    } else if (value) {
      setMode("advanced");
    }
  }, [onConditionDraftsChange, triggerType, value]);

  const updateConditions = (next: ParsedConditionExpression[]) => {
    onConditionDraftsChange(next);
    if (
      next.some((condition) =>
        conditionValueError(condition.field, condition.value),
      )
    ) {
      return;
    }
    const expression = buildConditionExpressions(next);
    localValue.current = expression;
    onChange(expression);
  };

  const fields = conditionFieldsForTrigger(triggerType);
  const fullHeightPickerExpanded =
    mode === "basic" && fieldUsesFullHeightPicker(expandedField ?? "");

  return (
    <Tabs
      className={cn(
        "space-y-3",
        fullHeightPickerExpanded &&
          "flex h-full min-h-0 flex-col space-y-0 gap-3",
      )}
      onValueChange={(next) => setMode(next as "basic" | "advanced")}
      value={mode}
    >
      <TabsList
        aria-label="Condition editor mode"
        className="grid h-9 w-full grid-cols-2 p-0.5"
      >
        <TabsTrigger className="h-8" disabled={disabled} value="basic">
          Basic
        </TabsTrigger>
        <TabsTrigger className="h-8" disabled={disabled} value="advanced">
          Advanced
        </TabsTrigger>
      </TabsList>

      {mode === "advanced" ? (
        <div className="space-y-2">
          <Input
            aria-label="Advanced expression"
            autoCapitalize="off"
            autoCorrect="off"
            disabled={disabled}
            onChange={(event) => {
              onConditionDraftsChange(null);
              localValue.current = event.target.value;
              onChange(event.target.value.trim());
            }}
            placeholder='e.g. trigger_author == "…"'
            value={value}
          />
          <p className="text-xs text-muted-foreground">
            Use an evalexpr expression. Existing custom expressions stay
            unchanged.
          </p>
        </div>
      ) : parsedValue === null && value.trim() ? (
        <div className="space-y-3 rounded-md border border-border/70 p-3">
          <p className="text-xs text-muted-foreground">
            An advanced expression is active. Replacing it with basic filters
            cannot be undone.
          </p>
          <button
            className="text-sm font-medium text-destructive hover:underline"
            disabled={disabled}
            onClick={() => {
              onConditionDraftsChange(null);
              localValue.current = "";
              onChange("");
            }}
            type="button"
          >
            Replace with basic filters
          </button>
        </div>
      ) : (
        <div
          className={cn(
            "divide-y divide-border/50",
            fullHeightPickerExpanded && "flex min-h-0 flex-1 flex-col",
          )}
        >
          {fields.map((field) => {
            const existing = conditions.find(
              (condition) => condition.field === field.value,
            );
            const condition = existing ?? emptyCondition(field.value);
            const expanded = expandedField === field.value;
            const error = conditionValueError(field.value, condition.value);
            const summary = existing
              ? `${OPERATOR_LABELS[condition.operator]}${condition.value ? ` ${compact(condition.value)}` : ""}`
              : "Any";
            const authorSummary =
              existing &&
              field.value === "trigger_author" &&
              triggerPresentation.pubkey &&
              triggerPresentation.label
                ? triggerPresentation.label
                : null;
            const messageSummary =
              existing &&
              field.value === "trigger_message_id" &&
              triggerPresentation.messageId &&
              triggerPresentation.messageLabel
                ? triggerPresentation.messageLabel
                    .trim()
                    .replaceAll(/\s+/g, " ")
                : null;
            return (
              <div
                className={cn(
                  expanded && "pb-4",
                  expanded &&
                    fieldUsesFullHeightPicker(field.value) &&
                    "flex min-h-0 flex-1 flex-col",
                )}
                key={field.value}
              >
                <button
                  aria-expanded={expanded}
                  className="flex min-h-12 w-full items-center gap-3 py-3 text-left transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:opacity-50"
                  disabled={disabled}
                  onClick={() =>
                    setExpandedField(expanded ? null : field.value)
                  }
                  type="button"
                >
                  <span className="min-w-0 flex-1 truncate text-base font-medium">
                    {field.label}
                  </span>
                  {authorSummary ? (
                    <span className="flex min-w-0 max-w-44 items-center gap-1.5 text-xs text-muted-foreground">
                      <UserAvatar
                        avatarUrl={triggerPresentation.avatarUrl}
                        className="h-4 w-4 shrink-0"
                        displayName={authorSummary}
                        fallbackDelayMs={0}
                        size="xs"
                      />
                      <span className="truncate">{authorSummary}</span>
                    </span>
                  ) : messageSummary ? (
                    <span className="max-w-44 truncate text-xs text-muted-foreground">
                      “{messageSummary}”
                    </span>
                  ) : (
                    <span className="max-w-44 truncate font-mono text-xs text-muted-foreground">
                      {summary}
                    </span>
                  )}
                  <ChevronRight
                    className={cn(
                      "h-4 w-4 shrink-0 text-muted-foreground/70 transition-transform duration-150 motion-reduce:transition-none",
                      expanded && "rotate-90",
                    )}
                  />
                </button>
                {expanded ? (
                  <div
                    className={cn(
                      "animate-in space-y-3 pt-1 fade-in slide-in-from-top-1 duration-150 motion-reduce:animate-none",
                      fieldUsesFullHeightPicker(field.value) &&
                        "flex min-h-0 flex-1 flex-col space-y-0 gap-3",
                    )}
                  >
                    <fieldset>
                      <legend className="sr-only">Match</legend>
                      <div className="grid grid-cols-2 gap-2.5">
                        {conditionOperatorsForField(field.value).map(
                          (operator) => {
                            const selected = condition.operator === operator;
                            return (
                              <button
                                aria-pressed={selected}
                                className={cn(
                                  "flex min-h-12 items-center justify-center rounded-lg border px-3 py-2 text-center text-sm font-medium",
                                  "outline-2 outline-offset-2 outline-transparent transition-[background-color,border-color,color,outline-color] focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
                                  selected
                                    ? "border-border/0 bg-transparent text-foreground outline-foreground/45"
                                    : "border-border/70 bg-background/35 text-muted-foreground hover:border-border hover:bg-muted/55 hover:text-foreground hover:outline-muted-foreground/20",
                                )}
                                disabled={disabled}
                                key={operator}
                                onClick={() => {
                                  const next = { ...condition, operator };
                                  updateConditions([
                                    ...conditions.filter(
                                      (item) => item.field !== field.value,
                                    ),
                                    next,
                                  ]);
                                }}
                                type="button"
                              >
                                {OPERATOR_LABELS[operator]}
                              </button>
                            );
                          },
                        )}
                      </div>
                    </fieldset>
                    {conditionOperatorNeedsValue(condition.operator) ? (
                      field.value === "trigger_emoji" ? (
                        <WorkflowEmojiField
                          ariaLabel="Choose trigger emoji"
                          clearAriaLabel="Clear trigger emoji"
                          disabled={disabled}
                          id="wf-trigger-emoji"
                          onChange={(emoji) =>
                            updateConditions([
                              ...conditions.filter(
                                (item) => item.field !== field.value,
                              ),
                              { ...condition, value: emoji ?? "" },
                            ])
                          }
                          value={condition.value}
                        />
                      ) : field.value === "trigger_author" ? (
                        <div className="flex min-h-0 flex-1 flex-col gap-1.5">
                          <FieldLabel htmlFor="wf-trigger-author-value">
                            {field.label}
                          </FieldLabel>
                          <WorkflowAuthorPicker
                            channelId={workflowChannelId}
                            disabled={disabled}
                            id="wf-trigger-author-value"
                            onEscape={() => setExpandedField(null)}
                            onChange={(pubkey) =>
                              updateConditions([
                                ...conditions.filter(
                                  (item) => item.field !== field.value,
                                ),
                                { ...condition, value: pubkey },
                              ])
                            }
                            value={condition.value}
                          />
                        </div>
                      ) : field.value === "trigger_message_id" ? (
                        <div className="flex min-h-0 flex-1 flex-col gap-1.5">
                          <FieldLabel htmlFor="wf-trigger-message-id-value">
                            {field.label}
                          </FieldLabel>
                          <WorkflowMessagePicker
                            channelId={workflowChannelId}
                            disabled={disabled}
                            id="wf-trigger-message-id-value"
                            key={workflowChannelId ?? "unscoped"}
                            onEscape={() => setExpandedField(null)}
                            onChange={(messageId) =>
                              updateConditions([
                                ...conditions.filter(
                                  (item) => item.field !== field.value,
                                ),
                                { ...condition, value: messageId },
                              ])
                            }
                            value={condition.value}
                          />
                        </div>
                      ) : (
                        <div className="space-y-1.5">
                          <FieldLabel
                            htmlFor={`wf-trigger-${field.value}-value`}
                          >
                            {field.label}
                          </FieldLabel>
                          <Input
                            aria-describedby={
                              error
                                ? `wf-trigger-${field.value}-error`
                                : undefined
                            }
                            aria-invalid={error ? true : undefined}
                            autoCapitalize="off"
                            autoCorrect="off"
                            disabled={disabled}
                            id={`wf-trigger-${field.value}-value`}
                            onChange={(event) =>
                              updateConditions([
                                ...conditions.filter(
                                  (item) => item.field !== field.value,
                                ),
                                { ...condition, value: event.target.value },
                              ])
                            }
                            placeholder={fieldPlaceholder(field.value)}
                            spellCheck={field.value === "trigger_text"}
                            value={condition.value}
                          />
                          {error ? (
                            <p
                              className="text-xs text-destructive"
                              id={`wf-trigger-${field.value}-error`}
                            >
                              {error}
                            </p>
                          ) : null}
                        </div>
                      )
                    ) : null}
                    {existing ? (
                      <button
                        className="text-xs font-medium text-muted-foreground hover:text-foreground"
                        disabled={disabled}
                        onClick={() =>
                          updateConditions(
                            conditions.filter(
                              (item) => item.field !== field.value,
                            ),
                          )
                        }
                        type="button"
                      >
                        Clear filter
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
      )}
    </Tabs>
  );
}
