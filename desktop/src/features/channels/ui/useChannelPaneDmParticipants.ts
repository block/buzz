import * as React from "react";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import {
  getDmHuddleMemberPubkeys,
  hasOtherDmParticipant,
} from "@/features/channels/lib/dmHuddleMembers";
import type { ChannelPaneProps } from "./ChannelPane.types";

/** Derive the shared DM intro and huddle participants without changing identity. */
export function useChannelPaneDmParticipants({
  activeChannel,
  agentPubkeys,
  agentPubkeysPending,
  currentPubkey,
  profiles,
}: Pick<
  ChannelPaneProps,
  | "activeChannel"
  | "agentPubkeys"
  | "agentPubkeysPending"
  | "currentPubkey"
  | "profiles"
>) {
  const huddleMemberPubkeys = React.useMemo(
    () => getDmHuddleMemberPubkeys(activeChannel, agentPubkeys, currentPubkey),
    [activeChannel, agentPubkeys, currentPubkey],
  );
  const huddleMemberPubkeysPending =
    agentPubkeysPending && hasOtherDmParticipant(activeChannel, currentPubkey);
  const directMessageIntro = React.useMemo(
    () =>
      buildDirectMessageIntro({
        channel: activeChannel,
        currentPubkey,
        profiles,
      }),
    [activeChannel, currentPubkey, profiles],
  );
  return {
    directMessageIntro,
    huddleMemberPubkeys,
    huddleMemberPubkeysPending,
  };
}
