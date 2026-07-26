import type { ComponentProps } from "react";

import { MessageComposerToolbar } from "@/features/messages/ui/MessageComposerToolbar";
import { cn } from "@/shared/lib/cn";

type ComposerDockToolbarProps = ComponentProps<
  typeof MessageComposerToolbar
> & {
  accessoryVisible?: boolean;
};

/**
 * Keeps the composer dock's total height stable by trading a quiet-state spacer
 * for the equal-height activity rail outside the composer.
 */
export function ComposerDockToolbar({
  accessoryVisible,
  ...toolbarProps
}: ComposerDockToolbarProps) {
  return (
    <>
      {accessoryVisible !== undefined ? (
        <div
          aria-hidden="true"
          className={cn(
            "shrink-0 transition-[height] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
            accessoryVisible ? "h-0" : "h-3.5",
          )}
        />
      ) : null}
      <MessageComposerToolbar {...toolbarProps} />
    </>
  );
}
