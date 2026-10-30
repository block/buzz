import * as React from "react";

import {
  useArchiveChannelMutation,
  useChannelMembersQuery,
  useSetChannelTopicMutation,
  useUpdateChannelMutation,
} from "@/features/channels/hooks";
import { useChannelModerationCapabilities } from "@/features/channels/ui/ChannelManagementModerationActions";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { buildChannelAdminCommands } from "../lib/channelAdminCommands";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";

const NO_COMMANDS: readonly SlashCommandDefinition[] = [];

export function useChannelAdminCommands(
  environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  const { enabled, channelId, channelType } = environment;
  const inChannel = enabled && channelId !== null && channelType !== "dm";
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const membersQuery = useChannelMembersQuery(channelId, inChannel);
  const { canManageChannel } = useChannelModerationCapabilities(
    membersQuery.data,
    currentPubkey,
    inChannel,
  );
  const selfRole = currentPubkey
    ? membersQuery.data?.find(
        (member) =>
          normalizePubkey(member.pubkey) === normalizePubkey(currentPubkey),
      )?.role
    : undefined;
  const canSetTopic = selfRole !== "guest";
  const { mutateAsync: updateChannel } = useUpdateChannelMutation(channelId);
  const { mutateAsync: archiveChannel } = useArchiveChannelMutation(channelId);
  const { mutateAsync: setChannelTopic } =
    useSetChannelTopicMutation(channelId);

  return React.useMemo(
    () =>
      enabled
        ? buildChannelAdminCommands({
            canManageChannel: inChannel && canManageChannel,
            canSetTopic: inChannel && canSetTopic,
            renameChannel: (name) => updateChannel({ name }),
            archiveChannel: () => archiveChannel(),
            setTopic: (topic) => setChannelTopic({ topic }),
          })
        : NO_COMMANDS,
    [
      enabled,
      inChannel,
      canManageChannel,
      canSetTopic,
      updateChannel,
      archiveChannel,
      setChannelTopic,
    ],
  );
}
