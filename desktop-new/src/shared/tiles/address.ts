/**
 * A tile address: what a reference in a message actually *is*.
 *
 * The whole point is that a reference carries the identity of the thing it
 * points at, never the letters of its name. The existing client stores literal
 * `@Label` text plus a separate `label -> pubkey` map and re-matches by string
 * at send time; that is why two people with the same display name force a
 * 64-character key into the visible text, why a pasted label needs
 * network-fenced verification, and why editing a name silently unbinds a
 * recipient. An address has no second thing to drift out of agreement with.
 *
 * A display name is resolved *from* an address for presentation and is never
 * treated as its meaning. Renames and same-name identities therefore cost
 * nothing.
 */

/** The kinds of thing a tile may refer to in version 1. */
export const TILE_KINDS = ["person", "agent", "channel"] as const;

export type TileKind = (typeof TILE_KINDS)[number];

export type TileAddress = {
  kind: TileKind;
  /** Pubkey for a person or agent; channel id for a channel. */
  id: string;
};

export function isTileKind(value: string): value is TileKind {
  return (TILE_KINDS as readonly string[]).includes(value);
}

/**
 * The address's canonical string form, used in a message body and in the
 * editor's own serialization.
 *
 * `buzz://` entity links already exist across this codebase, so a reader that
 * knows nothing about tiles still receives something meaningful rather than
 * broken markup — the reference degrades to a readable link instead of
 * disappearing.
 */
export function formatTileAddress(address: TileAddress): string {
  return `buzz://${address.kind}/${address.id}`;
}

const ADDRESS_PATTERN = /^buzz:\/\/([a-z]+)\/([A-Za-z0-9._:-]+)$/;

/** Parses a canonical address, or returns null when the text is not one. */
export function parseTileAddress(value: string): TileAddress | null {
  const match = ADDRESS_PATTERN.exec(value.trim());
  if (!match) return null;
  const [, kind, id] = match;
  if (!isTileKind(kind) || id.length === 0) return null;
  return { kind, id };
}

/** Stable key for maps and React keys. Not a display value. */
export function tileAddressKey(address: TileAddress): string {
  return `${address.kind}:${address.id}`;
}

export function sameTileAddress(a: TileAddress, b: TileAddress): boolean {
  return a.kind === b.kind && a.id === b.id;
}
