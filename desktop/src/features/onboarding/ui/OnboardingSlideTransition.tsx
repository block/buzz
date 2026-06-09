import type * as React from "react";

import { cn } from "@/shared/lib/cn";

type OnboardingSlideTransitionProps = React.HTMLAttributes<HTMLDivElement> & {
  children: React.ReactNode;
  direction?: OnboardingTransitionDirection;
  effect?: OnboardingTransitionEffect;
  transitionKey: string;
};

export type OnboardingTransitionDirection = "forward" | "backward";
export type OnboardingTransitionEffect =
  | "line-slide"
  | "mask-reveal-up"
  | "none";

export function OnboardingSlideTransition({
  children,
  className,
  direction = "forward",
  effect = "line-slide",
  transitionKey,
  ...props
}: OnboardingSlideTransitionProps) {
  return (
    <div className="w-full" key={transitionKey} {...props}>
      <TransitionLine
        contentClassName={className}
        direction={direction}
        effect={effect}
      >
        {children}
      </TransitionLine>
    </div>
  );
}

function TransitionLine({
  children,
  contentClassName,
  direction,
  effect,
}: {
  children: React.ReactNode;
  contentClassName?: string;
  direction: OnboardingTransitionDirection;
  effect: OnboardingTransitionEffect;
}) {
  return (
    <div
      className="sprout-onboarding-transition-line flex w-full justify-center"
      data-onboarding-direction={direction}
      data-onboarding-effect={effect}
    >
      <div
        className={cn(
          "sprout-onboarding-transition-content w-full",
          contentClassName,
        )}
      >
        {children}
      </div>
    </div>
  );
}
