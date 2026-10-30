import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { deleteDraftEntry } from "@/features/messages/lib/useDrafts";
import type { Channel, ChannelType } from "@/shared/api/types";
import { executeSlashCommand } from "./lib/executeSlashCommand";
import {
  recognizeSlashCommand,
  type SlashInvocation,
} from "./lib/recognizeSlashCommand";
import { resolveChannelLinkIds } from "./lib/resolveChannelLinkIds";
import { requestSlashCommandUi } from "./lib/slashCommandUiRequests";
import type {
  SlashCommandDefinition,
  SlashCommandThreadContext,
} from "./lib/types";

type UseSlashCommandRunnerOptions = {
  commands: readonly SlashCommandDefinition[];
  channelId: string | null;
  channelName: string;
  channelType: ChannelType | null;
  channels: readonly Channel[];
  draftKey: string | null | undefined;
  getComposerRevision: () => number;
  getThreadContext: () => SlashCommandThreadContext | null;
  /** Waits for pending mention checks, then resolves `@` mentions in `text`. */
  resolveMentions: (text: string) => Promise<string[]>;
  /** Publishes through the composer's `onSend` and rejects on failure. */
  send: (
    content: string,
    threadContext: SlashCommandThreadContext | null,
  ) => Promise<void>;
  /** Clears the editor, attachments and mention state after success. */
  clearComposer: () => void;
};

/**
 * Recognizes and runs slash commands for one composer. A command holds the
 * composer in `isPending` while it runs, clears the draft only when it
 * succeeds, and leaves the draft and attachments in place on any failure.
 */
export function useSlashCommandRunner(options: UseSlashCommandRunnerOptions) {
  const navigation = useAppNavigation();
  const [isPending, setIsPending] = React.useState(false);
  const isPendingRef = React.useRef(false);
  const isMountedRef = React.useRef(true);
  const optionsRef = React.useRef(options);
  optionsRef.current = options;
  const navigationRef = React.useRef(navigation);
  navigationRef.current = navigation;

  React.useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const recognize = React.useCallback(
    (draft: string) =>
      recognizeSlashCommand(draft, optionsRef.current.commands),
    [],
  );

  const run = React.useCallback(
    async (
      invocation: SlashInvocation<SlashCommandDefinition>,
      hasAttachments: boolean,
    ) => {
      if (isPendingRef.current) return;
      const start = optionsRef.current;
      const { channelId } = start;
      const startRevision = start.getComposerRevision();
      isPendingRef.current = true;
      setIsPending(true);
      try {
        const result = await executeSlashCommand({
          invocation,
          hasAttachments,
          isDm: optionsRef.current.channelType === "dm",
          reportFailure: ({ title, description }) =>
            toast.error(title, description ? { description } : undefined),
          createContext: async (args) => {
            if (!channelId) throw new Error("No channel is selected.");
            const threadContext = start.getThreadContext();
            if (threadContext && !threadContext.parentEventId) {
              throw new Error("This thread is still loading. Try again.");
            }
            const mentionPubkeys = await start.resolveMentions(args);
            const latest = optionsRef.current;
            return {
              channelId,
              channelName: latest.channelName,
              channelType: latest.channelType,
              isDm: latest.channelType === "dm",
              threadRootId: threadContext?.threadHeadId ?? null,
              threadContext,
              args,
              mentionPubkeys,
              channelLinkIds: resolveChannelLinkIds(args, latest.channels),
              sendMessage: (content) =>
                optionsRef.current.send(content, threadContext),
              navigation: navigationRef.current,
              requestUi: requestSlashCommandUi,
              toast,
            };
          },
        });
        if (result !== "succeeded") return;
        const current = optionsRef.current;
        if (start.getComposerRevision() !== startRevision) return;
        if (isMountedRef.current && current.draftKey === start.draftKey) {
          current.clearComposer();
        } else if (start.draftKey) {
          deleteDraftEntry(start.draftKey);
        }
      } finally {
        isPendingRef.current = false;
        if (isMountedRef.current) setIsPending(false);
      }
    },
    [],
  );

  return { isPending, recognize, run };
}
