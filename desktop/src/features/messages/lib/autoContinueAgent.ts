import type { TimelineMessage } from "@/features/messages/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type AutoContinueAnchor = Pick<
  TimelineMessage,
  "signerPubkey" | "pubkey" | "author" | "tags"
>;

type ComputeAutoContinueAgentMentionsInput = {
  anchor: AutoContinueAnchor | null | undefined;
  currentPubkey: string | null | undefined;
  agentPubkeys: ReadonlySet<string> | null | undefined;
  existingMentionPubkeys: readonly string[];
};

function anchorSignerPubkey(anchor: AutoContinueAnchor): string | null {
  const raw = anchor.signerPubkey ?? anchor.pubkey ?? anchor.author;
  if (!raw) {
    return null;
  }
  const normalized = normalizePubkey(raw);
  return normalized.length > 0 ? normalized : null;
}

function anchorMentions(anchor: AutoContinueAnchor, pubkey: string): boolean {
  const tags = anchor.tags ?? [];
  return tags.some(
    (tag) => tag[0] === "p" && normalizePubkey(tag[1] ?? "") === pubkey,
  );
}

export function computeAutoContinueAgentMentions({
  anchor,
  currentPubkey,
  agentPubkeys,
  existingMentionPubkeys,
}: ComputeAutoContinueAgentMentionsInput): string[] {
  if (!anchor || !currentPubkey || !agentPubkeys || agentPubkeys.size === 0) {
    return [];
  }

  const self = normalizePubkey(currentPubkey);
  if (self.length === 0) {
    return [];
  }

  const agentPubkey = anchorSignerPubkey(anchor);
  if (!agentPubkey || !agentPubkeys.has(agentPubkey) || agentPubkey === self) {
    return [];
  }

  if (!anchorMentions(anchor, self)) {
    return [];
  }

  const alreadyMentioned = new Set(
    existingMentionPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  return alreadyMentioned.has(agentPubkey) ? [] : [agentPubkey];
}
