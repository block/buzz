import { IconHash, IconRobot, IconUser } from "@tabler/icons-react";
import { useSyncExternalStore } from "react";

import type { TileAddress, TileKind } from "@/shared/tiles/address";
import {
  TILE_KIND_TRIGGER,
  type TileFace,
  tileFaces,
} from "@/shared/tiles/faceResolver";

/**
 * A reference to a person, agent, or channel, shown inline in a sentence.
 *
 * One component serves both the composer and the conversation. Two
 * implementations of one face drift, and a reference that looks different
 * depending on whether it is being written or being read is the kind of
 * disconnected feature this client exists to avoid.
 *
 * A tile is an object, not styled text: the caret cannot sit inside it, one
 * deletion removes it whole, and it carries its identity rather than its name.
 * The label is resolved from the address at render time, so a rename updates
 * every tile without editing anyone's draft.
 */

const KIND_ICON: Record<TileKind, typeof IconUser> = {
  person: IconUser,
  agent: IconRobot,
  channel: IconHash,
};

/** Subscribes to face changes so a rename repaints without a document edit. */
function useTileFace(address: TileAddress): TileFace {
  return useSyncExternalStore(
    (listener) => tileFaces.subscribe(listener),
    () => tileFaces.get(address),
  );
}

export function InlineTile({
  address,
  interactive = true,
  onActivate,
}: {
  address: TileAddress;
  /**
   * Whether the tile is a control. Read-only contexts pass false so a tile
   * does not claim an interactive screen-reader stop it cannot honour.
   */
  interactive?: boolean;
  onActivate?: (address: TileAddress) => void;
}) {
  const face = useTileFace(address);
  const Icon = KIND_ICON[address.kind];
  const trigger = TILE_KIND_TRIGGER[address.kind];

  const content = (
    <>
      {face.avatarUrl ? (
        <img
          className="inline-tile-avatar"
          src={face.avatarUrl}
          alt=""
          aria-hidden="true"
        />
      ) : (
        <Icon className="inline-tile-icon" aria-hidden="true" />
      )}
      <span className="inline-tile-label">
        {trigger}
        {face.label}
      </span>
      {face.status ? (
        <span
          className="inline-tile-status"
          data-status={face.status}
          aria-hidden="true"
        />
      ) : null}
    </>
  );

  // A tile carries one accessible name that states what it refers to. Its
  // avatar, icon, and status dot are decorative — the label already says it,
  // and a second owner of the same name produces a duplicate reader stop.
  //
  // An unresolved face has no name to announce, and announcing an abbreviated
  // identity tells a screen-reader user nothing. Say the kind is unresolved
  // instead; the visible fallback is a recognition aid, not a name.
  const accessibleName = face.resolved
    ? `${accessibleKind(address.kind)} ${face.label}`
    : `Unresolved ${accessibleKind(address.kind).toLowerCase()}`;

  if (!interactive) {
    return (
      <span
        className="inline-tile"
        data-kind={address.kind}
        data-loading={face.loading || undefined}
        aria-label={accessibleName}
        role="img"
      >
        {content}
      </span>
    );
  }

  return (
    <button
      type="button"
      className="inline-tile"
      data-kind={address.kind}
      data-loading={face.loading || undefined}
      aria-label={accessibleName}
      // Keeps the caret in the editor. A control that takes focus moves the
      // caret out and silently breaks every keyboard behaviour after it.
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => onActivate?.(address)}
    >
      {content}
    </button>
  );
}

function accessibleKind(kind: TileKind): string {
  if (kind === "channel") return "Channel";
  if (kind === "agent") return "Agent";
  return "Person";
}
