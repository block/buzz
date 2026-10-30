// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
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
        aria-label={checked ? uiT("Completed task") : uiT("Incomplete task")}
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
