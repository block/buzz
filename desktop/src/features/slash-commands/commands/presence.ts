import * as React from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";

import { customEmojiQueryKey } from "@/features/custom-emoji/hooks";
import { useSetUserStatusMutation } from "@/features/user-status/hooks";
import type { UserStatusInput } from "@/features/user-status/types";
import { listCustomEmoji } from "@/shared/api/customEmoji";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { PresenceStatus } from "@/shared/api/types";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import {
  SLASH_UI_OPEN_STATUS_DIALOG,
  SLASH_UI_SET_PRESENCE,
  type SetPresenceRequest,
} from "../lib/presenceUiRequests";
import { SlashUsageError } from "../lib/slashUsageError";
import { standardEmojiForShortcode } from "../lib/standardEmojiShortcode";
import { splitStatusArgs, startsWithShortcode } from "../lib/splitStatusArgs";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
  SlashCommandRunContext,
} from "../lib/types";

async function setPresence(
  context: SlashCommandRunContext,
  status: PresenceStatus,
  message: string,
) {
  if (context.args) throw new SlashUsageError();
  const request: SetPresenceRequest = { status, result: null };
  if (!context.requestUi(SLASH_UI_SET_PRESENCE, request) || !request.result) {
    throw new Error("Presence can't be changed right now.");
  }
  await request.result;
  context.toast.success(message);
}

async function runStatus(
  context: SlashCommandRunContext,
  queryClient: QueryClient,
  setUserStatus: (status: UserStatusInput) => Promise<unknown>,
) {
  const { args } = context;
  if (!args) {
    if (!context.requestUi(SLASH_UI_OPEN_STATUS_DIALOG)) {
      throw new Error("The status dialog can't be opened right now.");
    }
    return;
  }
  if (args.toLowerCase() === "clear") {
    await setUserStatus({ text: "", emoji: "" });
    context.toast.success("Status cleared");
    return;
  }
  const customEmoji: CustomEmoji[] = startsWithShortcode(args)
    ? await queryClient.ensureQueryData({
        queryKey: customEmojiQueryKey,
        queryFn: listCustomEmoji,
      })
    : [];
  const { emoji, text } = splitStatusArgs(args, {
    customEmoji,
    standardEmoji: standardEmojiForShortcode,
  });
  await setUserStatus({ text, emoji });
  context.toast.success("Status set");
}

export function usePresenceCommands(
  _environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  const queryClient = useQueryClient();
  const identity = useIdentityQuery();
  const { mutateAsync: setUserStatus } = useSetUserStatusMutation(
    identity.data?.pubkey,
  );

  return React.useMemo<readonly SlashCommandDefinition[]>(
    () => [
      {
        name: "away",
        description: "Set yourself as away",
        usage: "/away",
        context: "any",
        run: (context) => setPresence(context, "away", "You're away"),
      },
      {
        name: "active",
        description: "Set yourself as active",
        usage: "/active",
        context: "any",
        run: (context) => setPresence(context, "online", "You're active"),
      },
      {
        name: "status",
        argumentHint: "[emoji] <text> | clear",
        description: "Set or clear your status",
        usage: "/status [emoji] <text> | /status clear",
        context: "any",
        run: (context) => runStatus(context, queryClient, setUserStatus),
      },
    ],
    [queryClient, setUserStatus],
  );
}
