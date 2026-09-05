import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

type OnboardingPreviewInputProps = React.ComponentProps<typeof Input> & {
  smooth?: boolean;
};

/**
 * Preview-only input wrapper that keeps the app's smooth-corner clipping on
 * the field while drawing the focus shadow on an unclipped outer frame.
 */
export const OnboardingPreviewInput = React.forwardRef<
  HTMLInputElement,
  OnboardingPreviewInputProps
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

  React.useEffect(() => {
    if (!smooth || !isFocused) {
      return;
    }

    const blurWhenPointerLeavesInput = (event: PointerEvent) => {
      const input = inputRef.current;
      if (!input || event.target === input) {
        return;
      }
      input.blur();
    };

    document.addEventListener("pointerdown", blurWhenPointerLeavesInput, true);
    return () => {
      document.removeEventListener(
        "pointerdown",
        blurWhenPointerLeavesInput,
        true,
      );
    };
  }, [isFocused, smooth]);

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
        className={cn(className, smooth && "focus-visible:shadow-none")}
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
OnboardingPreviewInput.displayName = "OnboardingPreviewInput";
