import type * as React from "react";

import { cn } from "@/shared/lib/cn";
import { readTaskIndex } from "@/shared/lib/rehypeTaskIndex";
import { Checkbox } from "@/shared/ui/checkbox";
import { useMarkdownRuntime } from "./runtimeContext";

type MarkdownInputProps = React.ComponentProps<"input"> & {
  node?: unknown;
};

/**
 * Renders the `<input>` remark-gfm generates for a task-list item.
 *
 * The checkbox is inert unless the surface opted in by supplying
 * `onToggleTask` on the Markdown runtime — read from context rather than
 * closed over, so the component map stays module-stable and the parsed tree
 * remains cacheable (see `runtimeContext.ts` and `nodeCache.ts`).
 *
 * Which task was clicked comes from the ordinal `rehypeTaskIndex` stamped on
 * the node: the generated `<input>` carries no source position, so there is
 * no offset to work back from. An unreadable ordinal keeps the checkbox
 * inert rather than guessing at a line to rewrite.
 */
export function MarkdownInput({
  checked,
  className,
  node: _node,
  type,
  ...props
}: MarkdownInputProps) {
  const { onToggleTask } = useMarkdownRuntime();

  if (type === "checkbox") {
    const isChecked = Boolean(checked);
    const taskIndex = readTaskIndex(
      (props as Record<string, unknown>)["data-task-index"],
    );
    const interactive = onToggleTask !== undefined && taskIndex !== null;

    return (
      <Checkbox
        aria-label={isChecked ? "Completed task" : "Incomplete task"}
        checked={isChecked}
        className={cn(
          "mr-1.5 inline-flex align-[-0.125rem] disabled:opacity-45",
          interactive ? "cursor-pointer" : "pointer-events-none",
          className,
        )}
        disabled={!interactive}
        onCheckedChange={
          interactive
            ? (next) => {
                onToggleTask(taskIndex, next === true);
              }
            : undefined
        }
        // The timeline row opens a thread on click; a toggle is not that.
        onClick={
          interactive
            ? (event: React.MouseEvent) => {
                event.stopPropagation();
              }
            : undefined
        }
        tabIndex={interactive ? 0 : -1}
      />
    );
  }

  return <input {...props} className={className} type={type} />;
}
