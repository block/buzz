import { ChevronRight } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

import { useMarkdownRuntime } from "./runtimeContext";

/**
 * Open sections, keyed by message id and the section key from
 * `remarkDetails`. Module-level so a reader's choice survives the timeline's
 * per-channel remount and in-place edits of the message (status boards);
 * cleared on app restart, collapsed being the default.
 */
const openSections = new Set<string>();

/** Community switches swap message spaces (see `resetCommunityState`). */
export function clearOpenMarkdownSections() {
  openSections.clear();
}

export function MarkdownDetails({
  children,
  detailsKey,
  interactive,
}: {
  children?: React.ReactNode;
  detailsKey?: string;
  interactive: boolean;
}) {
  const { messageId } = useMarkdownRuntime();
  const storeKey =
    messageId && detailsKey ? `${messageId}\u0000${detailsKey}` : null;
  const [open, setOpen] = React.useState(
    () => storeKey !== null && openSections.has(storeKey),
  );

  const [summary, ...body] = React.Children.toArray(children).filter(
    (child) => !(typeof child === "string" && child.trim() === ""),
  );

  const toggle = React.useCallback(() => {
    setOpen((value) => {
      const next = !value;
      if (storeKey !== null) {
        if (next) openSections.add(storeKey);
        else openSections.delete(storeKey);
      }
      return next;
    });
  }, [storeKey]);

  return (
    <div data-details="" data-open={open ? "true" : "false"}>
      <button
        aria-expanded={open}
        className={cn(
          "flex items-center gap-1 text-left font-medium",
          interactive ? "cursor-pointer hover:underline" : "cursor-default",
        )}
        disabled={!interactive}
        onClick={toggle}
        type="button"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn(
            "size-4 shrink-0 transition-transform motion-reduce:transition-none",
            open && "rotate-90",
          )}
        />
        <span>{summary}</span>
      </button>
      {open ? (
        <div className="mt-1.5 pl-5 [&>*+*]:mt-3 [&>ol]:space-y-conversation-list [&>ul]:space-y-conversation-list">
          {body}
        </div>
      ) : null}
    </div>
  );
}

/** The summary's children render inside the toggle button. */
export function MarkdownDetailsSummary({
  children,
}: {
  children?: React.ReactNode;
}) {
  return <>{children}</>;
}
