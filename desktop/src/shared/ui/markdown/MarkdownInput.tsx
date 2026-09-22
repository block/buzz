import type * as React from "react";

import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
  if (type === "checkbox") {
    return (
      <Checkbox
        aria-label={
          checked
            ? t("shared.markdown.task.completed-aria")
            : t("shared.markdown.task.incomplete-aria")
        }
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
