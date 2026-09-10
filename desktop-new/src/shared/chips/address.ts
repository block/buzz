/**
 * A chip address: what a reference in a message actually *is*.
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

/**
 * The kinds of thing a chip may refer to.
 *
 * The component understands five because the design draws five. Version 1's
 * picker only *offers* the first three — a message or link chip arrives by
 * another route (referencing a message, pasting a URL) rather than by typing a
 * trigger, so those kinds render before they are authorable.
 */
export const CHIP_KINDS = [
  "person",
  "agent",
  "channel",
  "message",
  "link",
] as const;

export type ChipKind = (typeof CHIP_KINDS)[number];

/** The kinds version 1's picker offers when someone types a trigger. */
export const AUTHORABLE_CHIP_KINDS: readonly ChipKind[] = [
  "person",
  "agent",
  "channel",
];

export type ChipAddress = {
  kind: ChipKind;
  /**
   * Pubkey for a person or agent, channel id for a channel, event id for a
   * message, URL for a link.
   */
  id: string;
};

export function isChipKind(value: string): value is ChipKind {
  return (CHIP_KINDS as readonly string[]).includes(value);
}

/**
 * The address's canonical string form, used in a message body and in the
 * editor's own serialization.
 *
 * `buzz://` entity links already exist across this codebase, so a reader that
 * knows nothing about chips still receives something meaningful rather than
 * broken markup — the reference degrades to a readable link instead of
 * disappearing.
 */
export function formatChipAddress(address: ChipAddress): string {
  return `buzz://${address.kind}/${address.id}`;
}

// A link's id is a URL, so the id segment is deliberately permissive: anything
// after the kind is the id, up to the end.
const ADDRESS_PATTERN = /^buzz:\/\/([a-z]+)\/(.+)$/;

/** Parses a canonical address, or returns null when the text is not one. */
export function parseChipAddress(value: string): ChipAddress | null {
  const match = ADDRESS_PATTERN.exec(value.trim());
  if (!match) return null;
  const [, kind, id] = match;
  if (!isChipKind(kind) || id.length === 0) return null;
  return { kind, id };
}

/** Stable key for maps and React keys. Not a display value. */
export function chipAddressKey(address: ChipAddress): string {
  return `${address.kind}:${address.id}`;
}

export function sameChipAddress(a: ChipAddress, b: ChipAddress): boolean {
  return a.kind === b.kind && a.id === b.id;
}
