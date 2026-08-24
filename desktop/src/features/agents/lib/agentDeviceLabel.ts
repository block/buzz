/**
 * Describes which computer an agent lives on, for a UI that must
 * distinguish same-named agents minted on different devices.
 *
 * The same account signed in on several computers mints a *separate*
 * keypair per computer for the same agent, so a channel can show four
 * identical "Winnie" entries of which only one is runnable here. This is
 * the copy that tells them apart.
 *
 * Returns `null` when there is nothing informative to say — a local agent
 * with no name collision is on this device by definition, and saying so
 * would be noise for the single-device majority.
 *
 * | isLocal | deviceLabel | hasNameCollision | result               |
 * | ------- | ----------- | ---------------- | -------------------- |
 * | true    | any         | false            | `null` (no noise)    |
 * | true    | any         | true             | `"on this device"`   |
 * | false   | `"mfeth-win"` | any            | `"on mfeth-win"`     |
 * | false   | null/empty  | any              | `"on another device"`|
 *
 * A whitespace-only label counts as absent. A wrong device name is worse
 * than no device name, so a missing label is never filled in with a guess.
 */
export function describeAgentDevice(input: {
  /** True when this pubkey has a record in the local managed-agent store. */
  isLocal: boolean;
  /** Device label read off the agent's kind:30177 event, if any. */
  deviceLabel?: string | null;
  /** True when another visible suggestion shares this display name. */
  hasNameCollision: boolean;
}): string | null {
  if (input.isLocal) {
    return input.hasNameCollision ? "on this device" : null;
  }

  const label = input.deviceLabel?.trim();
  return label ? `on ${label}` : "on another device";
}
