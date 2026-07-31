import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Agent-authored reply target the composer should auto-mention. */
export type ReplyTargetAgent = {
  /** Event id of the message being replied to. */
  targetId: string;
  /** Agent identity pubkey (normalized lowercase hex) — the key the ACP
   * harness gates on, not the raw signer pubkey. */
  pubkey: string;
  /** Name to render in the inserted `@Name` chip. */
  displayName: string;
};

/**
 * Resolve the effective reply target into an auto-mention descriptor, or null
 * when the target is absent, human-authored, or unresolvable. Agent detection
 * is layered the same way the thread panel already does it: the message's own
 * `isAgent` flag, the known-agent pubkey set, then the profile flag.
 */
export function resolveReplyTargetAgent(
  target:
    | {
        id: string;
        pubkey?: string;
        author?: string;
        isAgent?: boolean;
      }
    | null
    | undefined,
  knownAgentPubkeys: ReadonlySet<string> | null | undefined,
  profiles?: UserProfileLookup,
): ReplyTargetAgent | null {
  if (!target?.pubkey) {
    return null;
  }

  const pubkey = normalizePubkey(target.pubkey);
  if (!pubkey) {
    return null;
  }

  const profile = profiles?.[pubkey];
  const isAgent =
    target.isAgent === true ||
    knownAgentPubkeys?.has(pubkey) === true ||
    profile?.isAgent === true;
  if (!isAgent) {
    return null;
  }

  const displayName =
    profile?.displayName?.trim() ||
    profile?.name?.trim() ||
    target.author?.trim() ||
    "";
  if (!displayName) {
    return null;
  }

  return { targetId: target.id, pubkey, displayName };
}
