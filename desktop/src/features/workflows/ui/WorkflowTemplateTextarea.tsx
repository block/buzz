import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { Textarea } from "@/shared/ui/textarea";
import type { StepFormState, TriggerType } from "./workflowFormTypes";
import {
  activeTemplateToken,
  insertTemplateVariable,
  workflowTemplateVariables,
} from "./workflowTemplateVariables";
import type {
  ActiveTemplateToken,
  WorkflowTemplateVariable,
} from "./workflowTemplateVariables";

export function WorkflowTemplateTextarea({
  className,
  disabled,
  onValueChange,
  previousSteps,
  triggerType,
  value,
  ...props
}: Omit<React.ComponentProps<typeof Textarea>, "onChange" | "value"> & {
  onValueChange: (value: string) => void;
  previousSteps: StepFormState[];
  triggerType: TriggerType;
  value: string;
}) {
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);
  const generatedId = React.useId();
  const textareaId = props.id ?? `workflow-template-${generatedId}`;
  const listboxId = `${textareaId}-variables`;
  const [token, setToken] = React.useState<ActiveTemplateToken | null>(null);
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const variables = React.useMemo(
    () => workflowTemplateVariables(triggerType, previousSteps),
    [previousSteps, triggerType],
  );
  const suggestions = React.useMemo(() => {
    if (!token) return [];
    const query = token.query.toLowerCase();
    return variables.filter(
      (variable) =>
        variable.value.toLowerCase().includes(query) ||
        variable.description.toLowerCase().includes(query),
    );
  }, [token, variables]);
  const open = token !== null;
  const selectedOptionId = suggestions[selectedIndex]
    ? `${listboxId}-option-${selectedIndex}`
    : undefined;

  const updateToken = React.useCallback((nextValue: string, caret: number) => {
    setToken(activeTemplateToken(nextValue, caret));
    setSelectedIndex(0);
  }, []);

  const selectVariable = React.useCallback(
    (variable: WorkflowTemplateVariable) => {
      if (!token) return;
      const next = insertTemplateVariable(value, token, variable.value);
      onValueChange(next.value);
      setToken(null);
      requestAnimationFrame(() => {
        const textarea = textareaRef.current;
        textarea?.focus();
        textarea?.setSelectionRange(next.caret, next.caret);
      });
    },
    [onValueChange, token, value],
  );

  return (
    <Popover
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) setToken(null);
      }}
    >
      <PopoverAnchor asChild>
        <Textarea
          {...props}
          aria-activedescendant={open ? selectedOptionId : undefined}
          aria-autocomplete="list"
          aria-controls={open ? listboxId : undefined}
          aria-expanded={open}
          aria-haspopup="listbox"
          className={className}
          disabled={disabled}
          id={textareaId}
          onChange={(event) => {
            const nextValue = event.target.value;
            onValueChange(nextValue);
            updateToken(nextValue, event.target.selectionStart);
          }}
          onClick={(event) =>
            updateToken(
              event.currentTarget.value,
              event.currentTarget.selectionStart,
            )
          }
          onKeyDown={(event) => {
            if (!open) return;
            if (event.key === "Escape") {
              event.preventDefault();
              setToken(null);
              return;
            }
            if (suggestions.length === 0) return;
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              const direction = event.key === "ArrowDown" ? 1 : -1;
              setSelectedIndex(
                (current) =>
                  (current + direction + suggestions.length) %
                  suggestions.length,
              );
              return;
            }
            if (event.key === "Enter" || event.key === "Tab") {
              event.preventDefault();
              selectVariable(suggestions[selectedIndex] ?? suggestions[0]);
            }
          }}
          ref={textareaRef}
          value={value}
        />
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-[var(--radix-popover-trigger-width)] min-w-72 p-1.5"
        onOpenAutoFocus={(event) => event.preventDefault()}
        side="bottom"
        sideOffset={6}
      >
        <div className="max-h-72 overflow-y-auto" id={listboxId} role="listbox">
          {suggestions.length > 0 ? (
            suggestions.map((variable, index) => {
              const previousGroup = suggestions[index - 1]?.group;
              return (
                <React.Fragment key={variable.value}>
                  {variable.group !== previousGroup ? (
                    <p className="px-2 pb-1 pt-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                      {variable.group}
                    </p>
                  ) : null}
                  <button
                    aria-selected={index === selectedIndex}
                    className={cn(
                      "flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left",
                      index === selectedIndex
                        ? "bg-muted text-foreground"
                        : "text-foreground hover:bg-muted/60",
                    )}
                    id={`${listboxId}-option-${index}`}
                    onMouseDown={(event) => event.preventDefault()}
                    onMouseEnter={() => setSelectedIndex(index)}
                    onClick={() => selectVariable(variable)}
                    role="option"
                    type="button"
                  >
                    <code className="min-w-0 truncate text-xs">
                      {`{{${variable.value}}}`}
                    </code>
                    <span className="shrink-0 text-2xs text-muted-foreground">
                      {variable.description}
                    </span>
                  </button>
                </React.Fragment>
              );
            })
          ) : (
            <p className="px-2 py-2 text-xs text-muted-foreground">
              No matching variables
            </p>
          )}
        </div>
        {triggerType === "webhook" ? (
          <p className="border-t border-border/50 px-2 pb-1 pt-2 text-2xs text-muted-foreground">
            JSON fields are available as {"{{trigger.field_name}}"}.
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
