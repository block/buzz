import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

import { ONBOARDING_CARD_INPUT_CLASS } from "./onboardingCardStyles";

type OnboardingInputProps = React.ComponentProps<typeof Input> & {
  smooth?: boolean;
};

/**
 * Onboarding input with smooth clipping on the field and an unclipped focus
 * frame, matching the card treatment without changing the shared Input.
 */
export const OnboardingInput = React.forwardRef<
  HTMLInputElement,
  OnboardingInputProps
>(({ className, onBlur, onFocus, smooth = true, ...props }, forwardedRef) => {
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const [isFocused, setIsFocused] = React.useState(false);
  useSmoothCorners(inputRef, { enabled: smooth });

  const setInputRef = React.useCallback(
    (node: HTMLInputElement | null) => {
      inputRef.current = node;
      if (typeof forwardedRef === "function") {
        forwardedRef(node);
      } else if (forwardedRef) {
        forwardedRef.current = node;
      }
    },
    [forwardedRef],
  );

  return (
    <span
      className={cn(
        smooth
          ? "block rounded-xl transition-[box-shadow] duration-200 ease-out motion-reduce:transition-none"
          : "contents",
        smooth &&
          isFocused &&
          "shadow-[0_0_0_3px_white,0_0_0_6px_rgba(0,0,0,0.06)]",
      )}
      data-focused={isFocused ? "true" : undefined}
    >
      <Input
        className={cn(
          ONBOARDING_CARD_INPUT_CLASS,
          className,
          smooth && "focus-visible:shadow-none",
        )}
        onBlur={(event) => {
          setIsFocused(false);
          onBlur?.(event);
        }}
        onFocus={(event) => {
          setIsFocused(true);
          onFocus?.(event);
        }}
        ref={setInputRef}
        {...props}
      />
    </span>
  );
});
OnboardingInput.displayName = "OnboardingInput";
