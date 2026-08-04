import * as React from "react";

import { getMentionOffset } from "@/features/messages/lib/hasMention";
import {
  getPersistentAgentAudienceRevision,
  usePersistentAgentAudience,
} from "@/features/messages/lib/persistentAgentAudience";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import { truncatePubkey } from "@/shared/lib/pubkey";

const RECONCILE_DELAY_MS = 150;

export type PersistentMentionTarget = {
  displayName: string;
  pubkey: string;
};

export function resolvePersistentMentionTargets(
  pubkeys: Iterable<string>,
  getDisplayName: (pubkey: string) => string | null,
): PersistentMentionTarget[] {
  const targets = [...new Set(pubkeys)]
    .map((pubkey) => ({ pubkey, displayName: getDisplayName(pubkey) }))
    .filter(
      (target): target is PersistentMentionTarget =>
        target.displayName !== null,
    );
  const nameCounts = new Map<string, number>();
  for (const { displayName } of targets) {
    const key = displayName.trim().toLowerCase();
    nameCounts.set(key, (nameCounts.get(key) ?? 0) + 1);
  }
  return targets.map((target) =>
    (nameCounts.get(target.displayName.trim().toLowerCase()) ?? 0) > 1
      ? {
          ...target,
          displayName: `${target.displayName} (${truncatePubkey(target.pubkey)})`,
        }
      : target,
  );
}

export function getPersistentMentionTokenRemovalRange(
  text: string,
  pubkey: string,
  hydratedLabels: ReadonlyMap<string, string>,
  getDisplayName: (pubkey: string) => string | null,
): { from: number; to: number } | null {
  const displayName = hydratedLabels.get(pubkey) ?? getDisplayName(pubkey);
  if (!displayName) return null;
  const from = getMentionOffset(text, displayName);
  if (from === null) return null;
  let to = from + displayName.length + 1;
  if (text[to] === " ") to += 1;
  return { from, to };
}

export function usePersistentAgentMentionHydration({
  audienceScope,
  hydrationKey,
  initialAgentPubkeys,
  isEditing,
  mentions,
  richText,
}: {
  audienceScope: string | null;
  hydrationKey: string | null | undefined;
  initialAgentPubkeys?: readonly string[];
  isEditing: boolean;
  mentions: UseMentionsResult;
  richText: UseRichTextEditorResult;
}) {
  const audience = usePersistentAgentAudience(audienceScope);
  const {
    enabled: audienceEnabled,
    initialize,
    pubkeys: audiencePubkeys,
  } = audience;
  const {
    cancelMentionAutocomplete,
    clearMentions,
    extractMentionPubkeys,
    getMentionDisplayName,
    insertResolvedMention,
    registerMentionPubkey,
  } = mentions;
  const { getPlainTextAndCursor, replacePlainTextRange } = richText;
  const audienceRef = React.useRef(audience);
  audienceRef.current = audience;
  const scopeRef = React.useRef(audienceScope);
  scopeRef.current = audienceScope;
  const isEditingRef = React.useRef(isEditing);
  isEditingRef.current = isEditing;
  React.useEffect(() => {
    if (!audienceScope || !initialAgentPubkeys) return;
    initialize(initialAgentPubkeys);
  }, [audienceScope, initialize, initialAgentPubkeys]);
  const isRestoringRef = React.useRef(false);
  const isSubmittingRef = React.useRef(false);
  const isMentionOpenRef = React.useRef(mentions.isMentionOpen);
  isMentionOpenRef.current = mentions.isMentionOpen;
  const cancelHydrationAutocompleteRef = React.useRef(false);
  const hydratedRef = React.useRef(false);
  const hydratedMentionLabelsRef = React.useRef(new Map<string, string>());
  const reconcileTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const cancelReconcile = React.useCallback(() => {
    if (reconcileTimerRef.current === null) return;
    clearTimeout(reconcileTimerRef.current);
    reconcileTimerRef.current = null;
  }, []);

  React.useEffect(() => cancelReconcile, [cancelReconcile]);

  const hydrate = React.useCallback(() => {
    const capturedScope = audienceScope;
    if (
      !audienceEnabled ||
      !capturedScope ||
      isEditingRef.current ||
      audiencePubkeys.length === 0
    ) {
      hydratedMentionLabelsRef.current.clear();
      hydratedRef.current = true;
      return;
    }
    isRestoringRef.current = true;
    const current = getPlainTextAndCursor().text;
    const targets = resolvePersistentMentionTargets(
      audiencePubkeys,
      getMentionDisplayName,
    );
    hydratedMentionLabelsRef.current = new Map(
      targets.map((target) => [target.pubkey, target.displayName]),
    );
    for (const target of targets)
      registerMentionPubkey(target.displayName, target.pubkey, {
        isAgent: true,
      });
    if (scopeRef.current !== capturedScope) {
      isRestoringRef.current = false;
      return;
    }
    const present = new Set(extractMentionPubkeys(current));
    let prefixLength = 0;
    for (const target of targets.filter(
      (candidate) => !present.has(candidate.pubkey),
    )) {
      if (scopeRef.current !== capturedScope) break;
      const edit = insertResolvedMention({
        ...target,
        isAgent: true,
        replaceFromOffset: prefixLength,
        replaceToOffset: prefixLength,
      });
      cancelHydrationAutocompleteRef.current = true;
      replacePlainTextRange(
        edit.replaceFromOffset,
        edit.replaceToOffset,
        edit.insertText,
      );
      prefixLength += edit.insertText.length;
    }
    hydratedRef.current = scopeRef.current === capturedScope;
    isRestoringRef.current = false;
    if (cancelHydrationAutocompleteRef.current) {
      cancelHydrationAutocompleteRef.current = false;
      // Hydration is a programmatic transition, not an authored query. Cancel
      // only when its editor updates actually scheduled autocomplete work.
      cancelMentionAutocomplete();
    }
  }, [
    audienceEnabled,
    audiencePubkeys,
    audienceScope,
    cancelMentionAutocomplete,
    extractMentionPubkeys,
    getMentionDisplayName,
    getPlainTextAndCursor,
    insertResolvedMention,
    registerMentionPubkey,
    replacePlainTextRange,
  ]);

  const reconcile = React.useCallback(
    (text: string) => {
      if (
        !hydratedRef.current ||
        isRestoringRef.current ||
        isSubmittingRef.current ||
        isEditingRef.current
      )
        return;
      cancelReconcile();
      reconcileTimerRef.current = setTimeout(() => {
        reconcileTimerRef.current = null;
        if (
          !hydratedRef.current ||
          isRestoringRef.current ||
          isSubmittingRef.current ||
          isEditingRef.current ||
          isMentionOpenRef.current
        )
          return;
        const present = new Set(extractMentionPubkeys(text));
        for (const pubkey of audienceRef.current.pubkeys) {
          const hydratedLabel =
            hydratedMentionLabelsRef.current.get(pubkey) ??
            getMentionDisplayName(pubkey);
          if (
            !present.has(pubkey) &&
            (!hydratedLabel || getMentionOffset(text, hydratedLabel) === null)
          ) {
            audienceRef.current.removePubkey(pubkey);
          }
        }
      }, RECONCILE_DELAY_MS);
    },
    [cancelReconcile, extractMentionPubkeys, getMentionDisplayName],
  );

  const hydrateRef = React.useRef(hydrate);
  hydrateRef.current = hydrate;
  const scheduleHydration = React.useCallback(
    (cancelAutocomplete = false) =>
      requestAnimationFrame(() => {
        hydrateRef.current();
        if (cancelAutocomplete) cancelMentionAutocomplete();
      }),
    [cancelMentionAutocomplete],
  );
  React.useEffect(() => {
    void hydrationKey;
    cancelReconcile();
    hydratedRef.current = false;
    hydratedMentionLabelsRef.current.clear();
    const frame = scheduleHydration();
    return () => cancelAnimationFrame(frame);
  }, [cancelReconcile, hydrationKey, scheduleHydration]);

  const resolvePostSendContent = React.useCallback(
    (explicitAgentPubkeys: string[]) => {
      if (!audienceEnabled || !audienceScope || isEditingRef.current) return "";
      const orderedPubkeys = [
        ...new Set([...explicitAgentPubkeys, ...audiencePubkeys]),
      ];
      const targets = resolvePersistentMentionTargets(
        orderedPubkeys,
        getMentionDisplayName,
      );
      hydratedMentionLabelsRef.current = new Map(
        targets.map((target) => [target.pubkey, target.displayName]),
      );
      clearMentions();
      for (const target of targets) {
        registerMentionPubkey(target.displayName, target.pubkey, {
          isAgent: true,
        });
      }
      isRestoringRef.current = true;
      hydratedRef.current = true;
      return (
        targets.map((target) => `@${target.displayName}`).join(" ") +
        (targets.length > 0 ? " " : "")
      );
    },
    [
      audienceEnabled,
      audiencePubkeys,
      audienceScope,
      clearMentions,
      getMentionDisplayName,
      registerMentionPubkey,
    ],
  );

  const removeMentionToken = React.useCallback(
    (pubkey: string) => {
      const current = getPlainTextAndCursor().text;
      const range = getPersistentMentionTokenRemovalRange(
        current,
        pubkey,
        hydratedMentionLabelsRef.current,
        getMentionDisplayName,
      );
      if (!range) return;
      hydratedMentionLabelsRef.current.delete(pubkey);
      replacePlainTextRange(range.from, range.to, "");
      cancelMentionAutocomplete();
    },
    [
      cancelMentionAutocomplete,
      getMentionDisplayName,
      getPlainTextAndCursor,
      replacePlainTextRange,
    ],
  );

  const beginSubmit = React.useCallback(() => {
    cancelReconcile();
    isSubmittingRef.current = true;
  }, [cancelReconcile]);

  const endSubmit = React.useCallback(() => {
    isSubmittingRef.current = false;
    scheduleHydration(true);
  }, [scheduleHydration]);

  const getAudienceRevision = React.useCallback(
    () =>
      scopeRef.current
        ? getPersistentAgentAudienceRevision(scopeRef.current)
        : null,
    [],
  );

  const audienceChips = React.useMemo(
    () =>
      resolvePersistentMentionTargets(
        audiencePubkeys,
        (pubkey) => getMentionDisplayName(pubkey) ?? truncatePubkey(pubkey),
      ),
    [audiencePubkeys, getMentionDisplayName],
  );

  const removeAudienceMember = React.useCallback(
    (pubkey: string) => {
      removeMentionToken(pubkey);
      audience.removePubkey(pubkey);
    },
    [audience.removePubkey, removeMentionToken],
  );

  return {
    audience,
    audienceChipsProps: {
      audience: audienceChips,
      onRemove: removeAudienceMember,
    },
    audienceChips,
    beginSubmit,
    endSubmit,
    getAudienceRevision,
    reconcile,
    removeAudienceMember,
    resolvePostSendContent,
    scheduleHydration,
  };
}
