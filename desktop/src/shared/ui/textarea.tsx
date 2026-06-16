import * as React from "react";

import { cn } from "@/shared/lib/cn";

const Textarea = React.forwardRef<
  HTMLTextAreaElement,
  React.ComponentProps<"textarea">
>(({ className, ...props }, ref) => {
  return (
    <textarea
      autoCapitalize="none"
      autoCorrect="off"
      spellCheck={false}
      className={cn(
        "flex min-h-20 w-full rounded-lg border border-input/40 bg-background px-3 py-2 text-base transition-colors placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        className,
      )}
      ref={ref}
      {...props}
    />
  );
});
Textarea.displayName = "Textarea";

export { Textarea };
