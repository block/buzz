import type { ReactNode } from "react";

import { cn } from "@/shared/lib/cn";

export function OnboardingStepHeader({
  className,
  description,
  descriptionClassName,
  title,
}: {
  className?: string;
  description: ReactNode;
  descriptionClassName?: string;
  title: ReactNode;
}) {
  return (
    <div className={cn("w-full max-w-[500px] text-center", className)}>
      <h1 className="text-title font-normal text-foreground">{title}</h1>
      <p
        className={cn(
          "mx-auto mt-3 max-w-[440px] text-sm leading-6 text-foreground/80",
          descriptionClassName,
        )}
      >
        {description}
      </p>
    </div>
  );
}
