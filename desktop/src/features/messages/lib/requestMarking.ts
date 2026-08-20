/**
 * Decides whether an outgoing message creates a tracked NIP-AD obligation.
 *
 * Split out of the send flow so the composer's banner and the send path read
 * the same answer. They used to be separate: the send path silently derived a
 * target from the mention list, and nothing in the UI said so. The two
 * failures that came from that — an obligation created by "@agent thanks",
 * and a request silently untracked because two agents were mentioned — were
 * both invisible to the sender at the moment they could still be fixed.
 *
 * See docs/nips/NIP-AD.md "Request validity".
 */
export type RequestMarking =
  | {
      /** No agent mentioned: an ordinary message. */
      kind: "none";
    }
  | {
      /** Exactly one agent mentioned; the message will be marked. */
      kind: "tracked";
      targetPubkey: string;
    }
  | {
      /**
       * More than one agent mentioned. v1 tracks exactly one obligation per
       * request and will not guess, so the message goes out unmarked — which
       * the composer must say out loud rather than leave to be discovered.
       */
      kind: "multi-agent";
      targetPubkeys: string[];
    }
  | {
      /** The sender explicitly opted out of tracking. */
      kind: "opted-out";
    };

/**
 * `mentionedAgentPubkeys` must already be filtered to agents — a merely
 * `p`-mentioned human must never become a target, or any of them could close
 * the agent's obligation.
 */
export function decideRequestMarking(
  mentionedAgentPubkeys: readonly string[],
  optedOut: boolean,
): RequestMarking {
  if (mentionedAgentPubkeys.length === 0) {
    return { kind: "none" };
  }
  if (optedOut) {
    return { kind: "opted-out" };
  }
  const unique = [...new Set(mentionedAgentPubkeys)];
  if (unique.length > 1) {
    return { kind: "multi-agent", targetPubkeys: unique };
  }
  return { kind: "tracked", targetPubkey: unique[0] };
}

/**
 * The `agent` tag values to attach. Exactly one, or none — never several: the
 * shared verifier classifies a multi-target request as unsupported, so
 * emitting one would create a request nobody could ever discharge.
 */
export function requestAgentPubkeysFor(marking: RequestMarking): string[] {
  return marking.kind === "tracked" ? [marking.targetPubkey] : [];
}
