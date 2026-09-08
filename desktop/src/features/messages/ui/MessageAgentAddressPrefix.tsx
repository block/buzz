import * as React from "react";
import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { getAgentAddressMentionPubkeys } from "../lib/agentAddressMention.mjs";
import { getVisibleAgentAddressPubkeys } from "../lib/getVisibleAgentAddressPubkeys";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { InlineChip } from "@/shared/ui/InlineChip";

/** Resolve all literal competitors before hiding tag-backed address chips. */
export function useMessageAgentAddressPrefix({
  profiles,
  body,
  tags,
  mentionNames,
  mentionPubkeysByName,
  isKnownAgentPubkey,
}: {
  profiles?: UserProfileLookup;
  body: string;
  tags?: string[][];
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  isKnownAgentPubkey: (pubkey: string) => boolean;
}) {
  const pubkeys = React.useMemo(
    () =>
      getVisibleAgentAddressPubkeys(
        body,
        getAgentAddressMentionPubkeys(tags).filter(isKnownAgentPubkey),
        mentionPubkeysByName,
        mentionNames,
      ),
    [body, tags, isKnownAgentPubkey, mentionPubkeysByName, mentionNames],
  );
  return pubkeys.length > 0 ? (
    <MessageAgentAddressPrefix profiles={profiles} pubkeys={pubkeys} />
  ) : undefined;
}

/** Visible send-state prefix for recipients kept in the composer address tray. */
export function MessageAgentAddressPrefix({
  profiles,
  pubkeys,
}: {
  profiles?: UserProfileLookup;
  pubkeys: readonly string[];
}) {
  return (
    <>
      {pubkeys.map((pubkey) => {
        const profile = profiles?.[pubkey];
        const label =
          profile?.displayName?.trim() ||
          profile?.name?.trim() ||
          truncatePubkey(pubkey);
        return (
          <React.Fragment key={pubkey}>
            {/* biome-ignore lint/a11y/useValidAriaRole: UserProfilePopover uses role for agent classification, not as an ARIA attribute. */}
            <UserProfilePopover
              botIdenticonValue={label}
              pubkey={pubkey}
              role="bot"
              triggerElement="span"
            >
              <InlineChip
                className="agent-mention-highlight"
                data-mention=""
                icon="agent"
                interactive
              >
                {label}
                <AgentManagementMarker
                  pubkey={pubkey}
                  ownerPubkey={profile?.ownerPubkey}
                />
              </InlineChip>
            </UserProfilePopover>{" "}
          </React.Fragment>
        );
      })}
    </>
  );
}
