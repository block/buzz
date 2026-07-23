import type * as React from "react";

import { cn } from "@/shared/lib/cn";

type OnboardingSlideTransitionProps = React.HTMLAttributes<HTMLDivElement> & {
  children: React.ReactNode;
  containerClassName?: string;
  direction?: OnboardingTransitionDirection;
  effect?: OnboardingTransitionEffect;
  transitionKey: string;
};

export type OnboardingTransitionDirection = "forward" | "backward";
export type OnboardingTransitionEffect =
  | "fade"
  | "line-slide"
  | "mask-reveal-up"
  | "none";

export function OnboardingSlideTransition({
  children,
  className,
  containerClassName,
  direction = "forward",
  effect = "line-slide",
  transitionKey,
  ...props
}: OnboardingSlideTransitionProps) {
  return (
    <div
      className={cn("buzz-onboarding-slide w-full", containerClassName)}
      key={transitionKey}
      {...props}
    >
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
      className="buzz-onboarding-transition-line flex w-full justify-center"
      data-onboarding-direction={direction}
      data-onboarding-effect={effect}
    >
      <div
        className={cn(
          "buzz-onboarding-transition-content w-full",
          contentClassName,
        )}
      >
        {children}
      </div>
    </div>
  );
}
