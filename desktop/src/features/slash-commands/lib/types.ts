import type { toast } from "sonner";

import type { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { ChannelType } from "@/shared/api/types";

/** `channel` commands act on a non-DM channel or a thread inside one. */
export type SlashCommandContextKind = "any" | "channel";

export type SlashCommandNavigation = ReturnType<typeof useAppNavigation>;

export type SlashCommandThreadContext = {
  parentEventId: string | null;
  threadHeadId: string | null;
};

/**
 * What a group hook may read while building definitions. Hooks are free to
 * call their own React hooks (queries, mutations, navigation) and close over
 * the results in `isVisible` and `run`; when `enabled` is false they should
 * skip any fetching.
 */
export type SlashCommandEnvironment = {
  enabled: boolean;
  channelId: string | null;
  channelName: string;
  channelType: ChannelType | null;
  isThread: boolean;
};

export type SlashCommandRunContext = {
  channelId: string;
  channelName: string;
  channelType: ChannelType | null;
  isDm: boolean;
  /** Thread head the command was typed under, or null in the main composer. */
  threadRootId: string | null;
  threadContext: SlashCommandThreadContext | null;
  /** Text after the command token, trimmed. */
  args: string;
  /**
   * Pubkeys of the `@` mentions in `args`, resolved through the composer's
   * exact-key mention map. An ambiguous typed name fails the whole command
   * before `run` is called.
   */
  mentionPubkeys: readonly string[];
  /** Ids of the non-DM channels referenced as `#name` in `args`. */
  channelLinkIds: readonly string[];
  /**
   * Publishes `content` through the composer's own `onSend`, into the channel
   * or thread the command was typed in. Rejects when the publish fails. It
   * does not clear the draft; the executor does that after `run` resolves.
   */
  sendMessage: (content: string) => Promise<void>;
  navigation: SlashCommandNavigation;
  /**
   * Opens UI owned by a component outside the composer (a dialog, a panel).
   * Returns false when nothing is listening; throw an error then.
   * See `lib/slashCommandUiRequests.ts`.
   */
  requestUi: (name: string, payload?: unknown) => boolean;
  toast: typeof toast;
};

export type SlashCommandDefinition = {
  /** Lowercase, no leading slash. Letters, digits, `_` and `-` only. */
  name: string;
  /** Lowercase names that run this command but never appear in the list. */
  aliases?: readonly string[];
  /** Shown after the name in the list, for example `[text]`. */
  argumentHint?: string;
  /** One line, shown in the list. */
  description: string;
  /** Shown on a usage error, for example `/shrug [text]`. */
  usage: string;
  context: SlashCommandContextKind;
  /**
   * Return false to hide the command from the list when the user is known to
   * be denied. A typed command still runs, and the relay stays the authority.
   */
  isVisible?: () => boolean;
  /**
   * Resolve to succeed; the executor clears the draft and the command shows
   * its own success toast. Throw to fail and keep the draft: a
   * `SlashUsageError` shows `usage`, any other error shows its message.
   */
  run: (context: SlashCommandRunContext) => Promise<void> | void;
};

export type SlashCommandGroupHook = (
  environment: SlashCommandEnvironment,
) => readonly SlashCommandDefinition[];
