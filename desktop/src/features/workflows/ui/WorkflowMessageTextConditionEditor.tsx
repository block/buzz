import { ChevronRight } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  MESSAGE_TEXT_CONDITION_LABELS,
  MESSAGE_TEXT_CONDITION_OPERATORS,
  buildMessageTextCondition,
  messageTextConditionNeedsValue,
  parseMessageTextCondition,
} from "./workflowMessageTextCondition";
import type { MessageTextCondition } from "./workflowMessageTextCondition";

const DEFAULT_CONDITION: MessageTextCondition = {
  operator: "contains",
  value: "",
};

type EditorMode = "basic" | "advanced";

function conditionSummary(condition: MessageTextCondition): string {
  const label = MESSAGE_TEXT_CONDITION_LABELS[condition.operator].toLowerCase();
  if (!messageTextConditionNeedsValue(condition.operator)) return label;
  const value = condition.value.trim();
  return value ? `${label} “${value}”` : "Any";
}

export function WorkflowMessageTextCondition({
  allowAdvanced = true,
  disabled,
  onChange,
  value,
}: {
  allowAdvanced?: boolean;
  disabled?: boolean;
  onChange: (value: string) => void;
  value: string;
}) {
  const initialParsed = React.useRef(parseMessageTextCondition(value));
  const [mode, setMode] = React.useState<EditorMode>(() =>
    value && !initialParsed.current ? "advanced" : "basic",
  );
  const [expanded, setExpanded] = React.useState(
    () => !value || initialParsed.current !== null,
  );
  const [replaceAdvancedOpen, setReplaceAdvancedOpen] = React.useState(false);
  const [condition, setCondition] = React.useState<MessageTextCondition>(
    initialParsed.current ?? DEFAULT_CONDITION,
  );
  const [advancedDraft, setAdvancedDraft] = React.useState(value);
  const localValueRef = React.useRef(value);
  const parsed = value ? parseMessageTextCondition(value) : null;
  const hasUnsupportedExpression = Boolean(value) && parsed === null;

  React.useEffect(() => {
    if (value === localValueRef.current) return;
    localValueRef.current = value;
    setAdvancedDraft(value);
    const nextParsed = parseMessageTextCondition(value);
    if (nextParsed) {
      setCondition(nextParsed);
      setMode("basic");
      setExpanded(true);
    } else if (!value) {
      setCondition(DEFAULT_CONDITION);
      setMode("basic");
      setExpanded(true);
    } else {
      setMode("advanced");
      setExpanded(false);
    }
  }, [value]);

  const emit = (expression: string) => {
    localValueRef.current = expression;
    onChange(expression);
  };

  const updateCondition = (next: MessageTextCondition) => {
    setCondition(next);
    emit(buildMessageTextCondition(next) ?? "");
  };

  const openBasicEditor = () => {
    if (hasUnsupportedExpression) {
      setReplaceAdvancedOpen(true);
      return;
    }
    setExpanded((current) => !current);
  };

  return (
    <>
      <Tabs
        className="space-y-3"
        onValueChange={(nextMode) => {
          if (!allowAdvanced) return;
          const editorMode = nextMode as EditorMode;
          setMode(editorMode);
          setExpanded(false);
          if (editorMode === "advanced") setAdvancedDraft(value);
        }}
        value={allowAdvanced ? mode : "basic"}
      >
        {allowAdvanced ? (
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
        ) : null}

        {!allowAdvanced || mode === "basic" ? (
          <div>
            {hasUnsupportedExpression ? (
              <p className="border-b border-border/50 pb-3 text-xs text-muted-foreground">
                An advanced expression is active. Choosing a basic filter will
                replace it.
              </p>
            ) : null}
            <div className="divide-y divide-border/50">
              <div className={cn(expanded && "pb-4 last:pb-0")}>
                <button
                  aria-expanded={expanded}
                  className="flex min-h-12 w-full items-center gap-3 py-3 text-left transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={disabled}
                  onClick={openBasicEditor}
                  type="button"
                >
                  <span className="min-w-0 flex-1 truncate text-base font-medium">
                    Message text
                  </span>
                  <span className="max-w-40 truncate text-sm text-muted-foreground">
                    {hasUnsupportedExpression
                      ? "Any"
                      : conditionSummary(condition)}
                  </span>
                  <ChevronRight
                    className={cn(
                      "h-4 w-4 shrink-0 text-muted-foreground/70 transition-transform duration-150 motion-reduce:transition-none",
                      expanded && "rotate-90",
                    )}
                  />
                </button>

                {expanded && !hasUnsupportedExpression ? (
                  <div className="animate-in space-y-4 pt-1 fade-in slide-in-from-top-1 duration-150 motion-reduce:animate-none">
                    <fieldset>
                      <legend className="sr-only">Match</legend>
                      <div className="grid grid-cols-2 gap-2.5">
                        {MESSAGE_TEXT_CONDITION_OPERATORS.map((operator) => {
                          const id = `wf-trigger-filter-trigger-text-operator-${operator}`;
                          return (
                            <div className="relative" key={operator}>
                              <input
                                checked={condition.operator === operator}
                                className="peer sr-only"
                                disabled={disabled}
                                id={id}
                                name="wf-trigger-filter-trigger-text-operator"
                                onChange={() =>
                                  updateCondition({ ...condition, operator })
                                }
                                type="radio"
                                value={operator}
                              />
                              <label
                                className={cn(
                                  "flex min-h-12 cursor-pointer items-center justify-center rounded-lg border px-3 py-2 text-center text-sm font-medium",
                                  "outline-2 outline-offset-2 outline-transparent transition-[background-color,border-color,color,outline-color]",
                                  "peer-focus-visible:ring-2 peer-focus-visible:ring-ring",
                                  "peer-disabled:cursor-not-allowed peer-disabled:opacity-50",
                                  condition.operator === operator
                                    ? "border-border/0 bg-transparent text-foreground outline-foreground/45"
                                    : "border-border/70 bg-background/35 text-muted-foreground hover:border-border hover:bg-muted/55 hover:text-foreground hover:outline-muted-foreground/20",
                                )}
                                htmlFor={id}
                              >
                                {MESSAGE_TEXT_CONDITION_LABELS[
                                  operator
                                ].toLowerCase()}
                              </label>
                            </div>
                          );
                        })}
                      </div>
                    </fieldset>

                    {messageTextConditionNeedsValue(condition.operator) ? (
                      <Input
                        aria-label="Text to match"
                        autoCapitalize="off"
                        autoCorrect="off"
                        disabled={disabled}
                        id="wf-trigger-filter-trigger-text-value"
                        onChange={(event) =>
                          updateCondition({
                            ...condition,
                            value: event.target.value,
                          })
                        }
                        placeholder="e.g. deploy"
                        value={condition.value}
                      />
                    ) : null}
                  </div>
                ) : null}
              </div>
            </div>
          </div>
        ) : (
          <div className="animate-in space-y-2 fade-in duration-150 motion-reduce:animate-none">
            <Input
              aria-label="Advanced expression"
              autoCapitalize="off"
              autoCorrect="off"
              disabled={disabled}
              id="wf-trigger-filter-advanced-expression"
              onChange={(event) => {
                const expression = event.target.value;
                setAdvancedDraft(expression);
                emit(expression.trim());
              }}
              placeholder='e.g. str_contains(trigger_text, "deploy")'
              value={advancedDraft}
            />
            <p className="text-xs text-muted-foreground">
              Use an evalexpr expression with <code>trigger_text</code>.
            </p>
          </div>
        )}
      </Tabs>

      <AlertDialog
        onOpenChange={setReplaceAdvancedOpen}
        open={replaceAdvancedOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Replace advanced expression?</AlertDialogTitle>
            <AlertDialogDescription>
              Choosing a basic filter will replace the entire advanced
              expression. This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Keep advanced expression
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                onClick={() => {
                  setCondition(DEFAULT_CONDITION);
                  setAdvancedDraft("");
                  setExpanded(true);
                  emit("");
                }}
                type="button"
                variant="destructive"
              >
                Replace expression
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
