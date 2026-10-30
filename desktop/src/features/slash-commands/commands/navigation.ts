import * as React from "react";

import {
  useOpenDmMutation,
  useUpsertCachedChannel,
} from "@/features/channels/hooks";
import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { useKlipyCapability } from "@/features/gifs/useKlipyCapability";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MAX_DM_OTHER_PARTICIPANTS, parseDmArgs } from "../lib/parseDmArgs";
import { SlashUsageError } from "../lib/slashUsageError";
import {
  gifPickerSlashUiName,
  huddleSlashUiName,
  requestSlashCommandUiAndSettle,
} from "../lib/slashUiRequest";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";

export function useNavigationCommands(
  environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  const identityQuery = useIdentityQuery();
  const selfPubkey = identityQuery.data?.pubkey ?? null;
  const openDm = useOpenDmMutation().mutateAsync;
  const upsertCachedChannel = useUpsertCachedChannel();
  const sendToChannel = useSendMessageMutation(
    null,
    identityQuery.data,
  ).mutateAsync;
  const customEmoji = useCustomEmoji();
  const gifs = useKlipyCapability(environment.enabled);
  const { available: gifsAvailable, isPending: gifsPending } = gifs;
  const isThread = environment.isThread;

  return React.useMemo<readonly SlashCommandDefinition[]>(
    () => [
      {
        name: "dm",
        aliases: ["msg"],
        argumentHint: "@user… [message]",
        description:
          "Open a direct message, optionally sending a message first",
        usage: "/dm @user… [message]",
        context: "any",
        run: async ({
          args,
          channelId,
          mentionRanges,
          navigation,
          threadRootId,
        }) => {
          const { recipients, message } = parseDmArgs(args, mentionRanges);
          const self = selfPubkey ? normalizePubkey(selfPubkey) : null;
          const pubkeys = [...new Set(recipients.map(normalizePubkey))].filter(
            (pubkey) => pubkey.length > 0 && pubkey !== self,
          );
          if (pubkeys.length === 0) {
            throw new SlashUsageError("Mention who to message.");
          }
          if (pubkeys.length > MAX_DM_OTHER_PARTICIPANTS) {
            throw new Error(
              `A DM can include at most ${MAX_DM_OTHER_PARTICIPANTS} other people.`,
            );
          }

          const dm = await openDm({ pubkeys });
          await upsertCachedChannel(dm);
          if (message) {
            await sendToChannel({
              targetChannel: dm,
              content: message,
              mentionPubkeys: [],
              mediaTags: buildCustomEmojiTags(message, customEmoji),
            });
          }
          if (dm.id === channelId && !threadRootId) return;
          await navigation.goChannel(dm.id);
        },
      },
      {
        name: "huddle",
        description: "Start a huddle in this channel, or join the one running",
        usage: "/huddle",
        context: "any",
        run: ({ channelId, requestUi }) =>
          requestSlashCommandUiAndSettle(
            requestUi,
            huddleSlashUiName(channelId),
            {},
            "Huddles aren't available here.",
          ),
      },
      {
        name: "gif",
        aliases: ["giphy"],
        argumentHint: "[query]",
        description: "Search GIFs to add to your message",
        usage: "/gif [query]",
        context: "any",
        isVisible: () => gifsAvailable,
        run: ({ args, channelId, requestUi }) => {
          if (!gifsAvailable) {
            throw new Error(
              gifsPending
                ? "Checking for GIF support. Try again in a moment."
                : "GIF search isn't available in this community.",
            );
          }
          return requestSlashCommandUiAndSettle(
            requestUi,
            gifPickerSlashUiName(channelId, isThread),
            { query: args },
            "The GIF picker isn't available here.",
          );
        },
      },
    ],
    [
      customEmoji,
      gifsAvailable,
      gifsPending,
      isThread,
      openDm,
      selfPubkey,
      sendToChannel,
      upsertCachedChannel,
    ],
  );
}
