import { cn } from "@/shared/lib/cn";

export function TemplateStepIndicator({ step }: { step: 1 | 2 }) {
  return (
    <div
      aria-label={`Step ${step} of 2`}
      aria-valuemax={2}
      aria-valuemin={1}
      aria-valuenow={step}
      className="flex w-full gap-2"
      data-testid="template-step-indicator"
      role="progressbar"
    >
      {[1, 2].map((segment) => (
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
