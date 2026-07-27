import type { ComponentProps } from "react";

import { MessageComposerToolbar } from "@/features/messages/ui/MessageComposerToolbar";

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
          className="composer-dock-quiet-spacer shrink-0"
        />
      ) : null}
      <MessageComposerToolbar {...toolbarProps} />
    </>
  );
}
