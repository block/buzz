import { IconLink, IconMessageCircle } from "@tabler/icons-react";
import { useSyncExternalStore } from "react";

import type { ChipAddress, ChipKind } from "@/shared/chips/address";
import {
  CHIP_KIND_TRIGGER,
  type ChipFace,
  chipFaces,
} from "@/shared/chips/faceResolver";

import { PreviewCard } from "./PreviewCard";

/**
 * A reference to a person, agent, channel, message, or link, shown inline in a
 * sentence. A resolved chip composes PreviewCard; `onActivate` is reserved for
 * a surface that owns an explicit persistent view.
 */

const KIND_ICON: Partial<Record<ChipKind, typeof IconLink>> = {
  message: IconMessageCircle,
  link: IconLink,
};

/** Subscribes to face changes so a rename repaints without a document edit. */
function useChipFace(address: ChipAddress): ChipFace {
  return useSyncExternalStore(
    (listener) => chipFaces.subscribe(listener),
    () => chipFaces.get(address),
  );
}

export function InlineChip({
  address,
  interactive = true,
  onActivate,
}: {
  address: ChipAddress;
  /**
   * An inert rendering for places where even a preview would compete with the
   * surrounding interaction, such as a future editable document boundary.
   */
  interactive?: boolean;
  /** An explicit owner-provided action; previews never imply a deep-open. */
  onActivate?: (address: ChipAddress) => void;
}) {
  const face = useChipFace(address);
  const Icon = KIND_ICON[address.kind];
  const trigger = CHIP_KIND_TRIGGER[address.kind];
  const kind = accessibleKind(address.kind);

  const content = (
    <>
      {Icon ? <Icon className="inline-chip-icon" aria-hidden="true" /> : null}
      <span className="inline-chip-label">
        {trigger}
        {face.label}
      </span>
    </>
  );

  const accessibleName = face.resolved
    ? `${kind} ${face.label}`
    : `Unresolved ${kind.toLowerCase()}`;
  const state = face.loading
    ? "loading"
    : face.resolved
      ? "resolved"
      : "unresolved";

  // An unresolved reference has nothing truthful to preview or open. A chip
  // that invites an action and does nothing is worse than an inert reference.
  if (!interactive || !face.resolved) {
    return (
      <span
        className="inline-chip"
        data-kind={address.kind}
        data-state={state}
        aria-label={accessibleName}
        role="img"
      >
        {content}
      </span>
    );
  }

  return (
    <PreviewCard
      trigger={
        <button
          type="button"
          className="inline-chip"
          data-kind={address.kind}
          data-state={state}
          aria-label={accessibleName}
          onClick={() => onActivate?.(address)}
        >
          {content}
        </button>
      }
    >
      <span className="buzz-preview-card-kind text-body-sm text-tertiary">
        {kind}
      </span>
      <span className="buzz-preview-card-name text-body text-primary">
        {face.label}
      </span>
      {face.status ? (
        <span className="text-body-sm text-secondary">{face.status}</span>
      ) : null}
    </PreviewCard>
  );
}

function accessibleKind(kind: ChipKind): string {
  if (kind === "channel") return "Channel";
  if (kind === "agent") return "Agent";
  if (kind === "message") return "Message";
  if (kind === "link") return "Link";
  return "Person";
}
