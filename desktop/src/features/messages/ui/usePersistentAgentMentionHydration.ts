import * as React from "react";

import { usePersistentAgentAudience } from "@/features/messages/lib/persistentAgentAudience";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";

export function usePersistentAgentMentionHydration({
  audienceScope,
  hydrationKey,
  isEditing,
  mentions,
  richText,
}: {
  audienceScope: string | null;
  hydrationKey: string | null | undefined;
  isEditing: boolean;
  mentions: UseMentionsResult;
  richText: UseRichTextEditorResult;
}) {
  const audience = usePersistentAgentAudience(audienceScope);
  const audienceRef = React.useRef(audience);
  audienceRef.current = audience;
  const scopeRef = React.useRef(audienceScope);
  scopeRef.current = audienceScope;
  const isEditingRef = React.useRef(isEditing);
  isEditingRef.current = isEditing;
  const isRestoringRef = React.useRef(false);
  const isSubmittingRef = React.useRef(false);
  const hydratedRef = React.useRef(false);

  const hydrate = React.useCallback(() => {
    const capturedScope = audienceScope;
    if (
      !audience.enabled ||
      !capturedScope ||
      isEditingRef.current ||
      audience.pubkeys.length === 0
    ) {
      hydratedRef.current = true;
      return;
    }
    isRestoringRef.current = true;
    const current = richText.getPlainTextAndCursor().text;
    const targets = audience.pubkeys
      .map((pubkey) => ({
        pubkey,
        displayName: mentions.getMentionDisplayName(pubkey),
      }))
      .filter((target): target is { pubkey: string; displayName: string } =>
        Boolean(target.displayName),
      );
    for (const target of targets)
      mentions.registerMentionPubkey(target.displayName, target.pubkey, {
        isAgent: true,
      });
    if (scopeRef.current !== capturedScope) {
      isRestoringRef.current = false;
      return;
    }
    const present = new Set(mentions.extractMentionPubkeys(current));
    let prefixLength = 0;
    for (const target of targets.filter(
      (candidate) => !present.has(candidate.pubkey),
    )) {
      if (scopeRef.current !== capturedScope) break;
      const edit = mentions.insertResolvedMention({
        ...target,
        isAgent: true,
        replaceFromOffset: prefixLength,
        replaceToOffset: prefixLength,
      });
      richText.replacePlainTextRange(
        edit.replaceFromOffset,
        edit.replaceToOffset,
        edit.insertText,
      );
      prefixLength += edit.insertText.length;
    }
    hydratedRef.current = scopeRef.current === capturedScope;
    isRestoringRef.current = false;
  }, [audience.enabled, audience.pubkeys, audienceScope, mentions, richText]);

  const reconcile = React.useCallback(
    (text: string) => {
      if (
        !hydratedRef.current ||
        isRestoringRef.current ||
        isSubmittingRef.current ||
        isEditingRef.current
      )
        return;
      const present = new Set(mentions.extractMentionPubkeys(text));
      for (const pubkey of audienceRef.current.pubkeys) {
        if (!present.has(pubkey)) audienceRef.current.removePubkey(pubkey);
      }
    },
    [mentions.extractMentionPubkeys],
  );

  const scheduleHydration = React.useCallback(
    () => requestAnimationFrame(hydrate),
    [hydrate],
  );
  React.useEffect(() => {
    void hydrationKey;
    hydratedRef.current = false;
    const frame = scheduleHydration();
    return () => cancelAnimationFrame(frame);
  }, [hydrationKey, scheduleHydration]);

  return {
    audience,
    beginSubmit: () => {
      isSubmittingRef.current = true;
    },
    endSubmit: () => {
      isSubmittingRef.current = false;
      scheduleHydration();
    },
    reconcile,
    scheduleHydration,
  };
}
