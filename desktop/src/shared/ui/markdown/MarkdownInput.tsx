import type * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Checkbox } from "@/shared/ui/checkbox";

type MarkdownInputProps = React.ComponentProps<"input"> & {
  node?: unknown;
};

export function MarkdownInput({
  checked,
  className,
  node: _node,
  type,
  ...props
}: MarkdownInputProps) {
  if (type === "checkbox") {
    return (
      <Checkbox
        aria-label={checked ? "Completed task" : "Incomplete task"}
        checked={Boolean(checked)}
        className={cn(
          "pointer-events-none mr-1.5 inline-flex align-[-0.125rem] disabled:opacity-45",
          className,
        )}
        disabled
        tabIndex={-1}
      />
    );
  }

  return <input {...props} className={className} type={type} />;
}
