import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { Textarea } from "@/shared/ui/textarea";
import { FieldLabel, FormSelect } from "./workflowFormPrimitives";
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

type EditorMode = "structured" | "advanced";

export function WorkflowMessageTextCondition({
  disabled,
  onChange,
  value,
}: {
  disabled?: boolean;
  onChange: (value: string) => void;
  value: string;
}) {
  const initialParsed = React.useRef(parseMessageTextCondition(value));
  const [mode, setMode] = React.useState<EditorMode>(() =>
    value && !initialParsed.current ? "advanced" : "structured",
  );
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
    } else if (!value) {
      setCondition(DEFAULT_CONDITION);
      setMode("structured");
    } else if (value) {
      setMode("advanced");
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

  return (
    <div className="space-y-3">
      <Tabs
        onValueChange={(nextMode) => {
          const editorMode = nextMode as EditorMode;
          setMode(editorMode);
          if (editorMode === "advanced") {
            setAdvancedDraft(value);
          } else if (parsed) {
            setCondition(parsed);
          } else if (!value) {
            setCondition(DEFAULT_CONDITION);
          }
        }}
        value={mode}
      >
        <TabsList
          aria-label="Text condition editor mode"
          className="grid h-9 w-full grid-cols-2 p-0.5"
        >
          <TabsTrigger disabled={disabled} value="structured">
            Structured
          </TabsTrigger>
          <TabsTrigger disabled={disabled} value="advanced">
            Advanced
          </TabsTrigger>
        </TabsList>
      </Tabs>

      {mode === "structured" ? (
        hasUnsupportedExpression ? (
          <div className="space-y-3 rounded-lg border border-border/70 bg-background/35 p-3">
            <p className="text-xs text-muted-foreground">
              An advanced raw Evalexpr expression is active. It remains
              unchanged until you explicitly replace it.
            </p>
            <Button
              disabled={disabled}
              onClick={() => {
                setCondition(DEFAULT_CONDITION);
                setAdvancedDraft("");
                emit("");
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              Replace with structured condition
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="space-y-1.5">
              <FieldLabel htmlFor="wf-trigger-text-operator">
                Text comparison
              </FieldLabel>
              <FormSelect
                disabled={disabled}
                id="wf-trigger-text-operator"
                onChange={(operator) =>
                  updateCondition({
                    ...condition,
                    operator: operator as MessageTextCondition["operator"],
                  })
                }
                value={condition.operator}
              >
                {MESSAGE_TEXT_CONDITION_OPERATORS.map((operator) => (
                  <option key={operator} value={operator}>
                    {MESSAGE_TEXT_CONDITION_LABELS[operator]}
                  </option>
                ))}
              </FormSelect>
            </div>
            {messageTextConditionNeedsValue(condition.operator) ? (
              <div className="space-y-1.5">
                <FieldLabel htmlFor="wf-trigger-text-value">
                  Text to match
                </FieldLabel>
                <Input
                  autoCapitalize="off"
                  autoCorrect="off"
                  disabled={disabled}
                  id="wf-trigger-text-value"
                  onChange={(event) =>
                    updateCondition({
                      ...condition,
                      value: event.target.value,
                    })
                  }
                  value={condition.value}
                />
              </div>
            ) : null}
            <p className="text-xs text-muted-foreground">
              Empty conditions match all messages.
            </p>
          </div>
        )
      ) : (
        <div className="space-y-1.5">
          <FieldLabel htmlFor="wf-trigger-filter">
            Raw Evalexpr expression
          </FieldLabel>
          <Textarea
            aria-label="Raw Evalexpr expression"
            autoCapitalize="off"
            autoCorrect="off"
            disabled={disabled}
            id="wf-trigger-filter"
            onChange={(event) => {
              const expression = event.target.value;
              setAdvancedDraft(expression);
              emit(expression);
            }}
            placeholder='e.g. str_contains(trigger_text, "deploy")'
            rows={4}
            value={advancedDraft}
          />
          <p className="text-xs text-muted-foreground">
            Advanced expressions are preserved exactly as entered.
          </p>
        </div>
      )}
    </div>
  );
}
