import { PreviewCard as BasePreviewCard } from "@base-ui/react/preview-card";
import type { ReactElement, ReactNode } from "react";

export type PreviewCardProps = {
  /** The one interactive element that anchors and reveals this preview. */
  trigger: ReactElement;
  /** Context that supplements, but never replaces, the trigger's destination. */
  children: ReactNode;
};

/**
 * A non-modal, portal-rendered preview of a resolved object.
 *
 * Base UI owns the hover/focus timing, anchored positioning, viewport collision,
 * and portal lifecycle. The preview stays supplemental: it does not take focus
 * or contain an action of its own.
 */
export function PreviewCard({ trigger, children }: PreviewCardProps) {
  return (
    <BasePreviewCard.Root>
      <BasePreviewCard.Trigger render={trigger} delay={250} closeDelay={150} />
      <BasePreviewCard.Portal>
        <BasePreviewCard.Positioner side="bottom" align="start" sideOffset={8}>
          <BasePreviewCard.Popup className="buzz-preview-card" role="tooltip">
            {children}
          </BasePreviewCard.Popup>
        </BasePreviewCard.Positioner>
      </BasePreviewCard.Portal>
    </BasePreviewCard.Root>
  );
}
