import * as React from "react";

import {
  DEFAULT_REPLY_MENTION_SETTINGS,
  readStoredReplyMentionSettings,
  type ReplyMentionSettings,
  writeStoredReplyMentionSettings,
} from "@/features/messages/lib/replyMentionSettings";

/**
 * React binding over the per-account reply-mention settings stored in
 * localStorage (see `lib/replyMentionSettings.ts`). Settings screens consume
 * this hook; the message send paths read the same storage directly via
 * `readStoredReplyMentionSettings` so a fresh value is picked up at send time.
 */
export function useReplyMentionSettings(pubkey?: string) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const [settings, setSettings] = React.useState<ReplyMentionSettings>(() =>
    readStoredReplyMentionSettings(normalizedPubkey),
  );

  React.useEffect(() => {
    setSettings(readStoredReplyMentionSettings(normalizedPubkey));
  }, [normalizedPubkey]);

  React.useEffect(() => {
    writeStoredReplyMentionSettings(normalizedPubkey, settings);
  }, [normalizedPubkey, settings]);

  const setAutoMentionRepliedTo = React.useCallback((enabled: boolean) => {
    setSettings((current) => ({
      ...current,
      autoMentionRepliedTo: enabled,
    }));
  }, []);

  const setMentionPrefixPubkeys = React.useCallback((pubkeys: string[]) => {
    setSettings((current) => ({
      ...current,
      mentionPrefixPubkeys: pubkeys,
    }));
  }, []);

  const resetReplyMentionSettings = React.useCallback(() => {
    setSettings(DEFAULT_REPLY_MENTION_SETTINGS);
  }, []);

  return {
    settings,
    setAutoMentionRepliedTo,
    setMentionPrefixPubkeys,
    resetReplyMentionSettings,
  };
}
