import { LayoutList, MessageSquare } from "lucide-react";

import type { TemplateType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

const TEMPLATE_TYPE_OPTIONS: Array<{
  type: TemplateType;
  title: string;
  description: string;
  icon: typeof MessageSquare;
}> = [
  {
    type: "channel",
    title: "Channel",
    description: "Set defaults for one new channel.",
    icon: MessageSquare,
  },
  {
    type: "section",
    title: "Section",
    description:
      "Set defaults inherited by every channel created in a section using this template.",
    icon: LayoutList,
  },
];

export function TemplateTypeStep({
  value,
  onChange,
  disabled,
}: {
  value: TemplateType;
  onChange: (value: TemplateType) => void;
  disabled: boolean;
}) {
  return (
    <div
      aria-label="Template type"
      className="grid grid-cols-2 gap-3"
      data-testid="template-type-options"
      role="radiogroup"
    >
      {TEMPLATE_TYPE_OPTIONS.map((option) => {
        const Icon = option.icon;
        const selected = option.type === value;
        return (
          <label
            className={cn(
              "relative flex min-h-48 flex-col items-start rounded-2xl border p-5 text-left transition-colors",
              selected
                ? "border-primary bg-background"
                : "border-border/70 bg-background",
              "hover:bg-input/30",
              "has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring has-[:focus-visible]:ring-offset-2",
              disabled && "pointer-events-none opacity-50",
            )}
            key={option.type}
          >
            <input
              checked={selected}
              className="sr-only"
              data-testid={`template-type-${option.type}`}
              disabled={disabled}
              name="template-type"
              onChange={() => onChange(option.type)}
              type="radio"
              value={option.type}
            />
            <span
              aria-hidden
              className={cn(
                "absolute right-4 top-4 flex size-4 items-center justify-center rounded-full border",
                selected
                  ? "border-primary"
                  : "border-muted-foreground/50 bg-background",
              )}
            >
              {selected ? (
                <span className="size-2 rounded-full bg-primary" />
              ) : null}
            </span>
            <span
              className="flex size-10 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-muted/20 text-muted-foreground"
              data-testid={`template-type-icon-${option.type}`}
            >
              <Icon aria-hidden className="size-5" />
            </span>
            <span
              className="mt-auto block pt-6"
              data-testid={`template-type-copy-${option.type}`}
            >
              <span className="block text-sm font-medium text-foreground">
                {option.title}
              </span>
              <span className="mt-1 block text-sm leading-5 text-muted-foreground">
                {option.description}
              </span>
            </span>
          </label>
        );
      })}
    </div>
  );
}

export function TemplateStepIndicator({ step }: { step: 1 | 2 | 3 }) {
  return (
    <div
      aria-label={`Step ${step} of 3`}
      className="flex w-full gap-2"
      data-testid="template-step-indicator"
      role="progressbar"
      aria-valuemax={3}
      aria-valuemin={1}
      aria-valuenow={step}
    >
      {[1, 2, 3].map((segment) => (
        <span
          className={cn(
            "h-1.5 flex-1 rounded-full",
            segment <= step ? "bg-primary" : "bg-input",
          )}
          data-state={segment <= step ? "active" : "future"}
          key={segment}
        />
      ))}
    </div>
  );
}
