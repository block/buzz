import { cn } from "@/shared/lib/cn";

type StepProgressProps = {
  activeSegmentClassName?: string;
  className?: string;
  completeSegmentClassName?: string;
  currentStep: number;
  inactiveSegmentClassName?: string;
  totalSteps?: number;
};

export function StepProgress({
  activeSegmentClassName,
  className,
  completeSegmentClassName,
  currentStep,
  inactiveSegmentClassName,
  totalSteps = 5,
}: StepProgressProps) {
  const safeTotalSteps = Math.max(1, totalSteps);
  const safeCurrentStep = Math.min(Math.max(1, currentStep), safeTotalSteps);

  return (
    <div
      aria-label={`Step ${safeCurrentStep} of ${safeTotalSteps}`}
      aria-valuemax={safeTotalSteps}
      aria-valuemin={1}
      aria-valuenow={safeCurrentStep}
      className={cn("flex items-center justify-center gap-1.5", className)}
      role="progressbar"
    >
      {Array.from({ length: safeTotalSteps }, (_, index) => {
        const step = index + 1;
        const isActive = step === safeCurrentStep;
        const isComplete = step < safeCurrentStep;

        return (
          <span
            aria-hidden="true"
            className={cn(
              "h-1.5 rounded-full transition-all duration-200 ease-out",
              isActive && cn("w-6 bg-primary", activeSegmentClassName),
              !isActive &&
                isComplete &&
                cn("w-1.5 bg-primary/35", completeSegmentClassName),
              !isActive &&
                !isComplete &&
                cn("w-1.5 bg-muted-foreground/25", inactiveSegmentClassName),
            )}
            key={step}
          />
        );
      })}
    </div>
  );
}
