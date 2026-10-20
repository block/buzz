import * as React from "react";
import { PRIVATE_CHANNEL_ADD_DENIED_MESSAGE } from "@/features/channels/lib/channelMemberAdmission";
import type { MentionRevalidationOptions } from "@/features/messages/lib/agentMentionRevalidation";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  MENTION_REFERENCE_TAG,
  mentionRevalidationOptions,
  uniqueNormalizedPubkeys,
  type PendingNonMemberMentionSend,
  type ComposerDraftOwner,
} from "./useMentionSendFlow.helpers";

/** Own the whole Invite attempt, not just the membership mutation's pending state. */
export function useNonMemberInvite({
  sourceOwner,
  draft,
  canInvite,
  revalidate,
  getManagedAgentsByPubkey,
  isAgentPubkey,
  addMembers,
  completeSend,
  setError,
}: {
  sourceOwner: ComposerDraftOwner;
  draft: PendingNonMemberMentionSend | null;
  canInvite: boolean;
  revalidate: (
    pubkeys: readonly string[],
    channelId?: string | null,
    options?: MentionRevalidationOptions,
  ) => Promise<string[]>;
  getManagedAgentsByPubkey: () => Promise<Map<string, ManagedAgent>>;
  isAgentPubkey: (pubkey: string) => boolean;
  addMembers: (input: {
    channelId?: string;
    pubkeys: string[];
    role: "member" | "bot";
  }) => Promise<{ errors: { error: string }[] }>;
  completeSend: (
    draft: PendingNonMemberMentionSend,
    pubkeys: string[],
    tags?: string[][],
  ) => Promise<void>;
  setError: (error: string | null) => void;
}) {
  const active = React.useRef<{
    controller: AbortController;
    draft: PendingNonMemberMentionSend;
  } | null>(null);
  const currentOwner = React.useRef(sourceOwner);
  currentOwner.current = sourceOwner;
  const [isPending, setIsPending] = React.useState(false);
  const cancel = React.useCallback(() => {
    active.current?.controller.abort();
    active.current = null;
    setIsPending(false);
  }, []);
  React.useLayoutEffect(() => {
    const attempt = active.current;
    // Clearing the prompt in completeSend is promotion, not cancellation.
    // A different non-null prompt or destination supersedes the old intent.
    if (
      attempt &&
      (attempt.draft.sourceOwner !== sourceOwner ||
        (draft !== null && draft !== attempt.draft))
    )
      cancel();
  }, [sourceOwner, draft, cancel]);
  React.useLayoutEffect(
    () => () => {
      active.current?.controller.abort();
      active.current = null;
    },
    [],
  );

  const invite = React.useCallback(() => {
    if (!draft || draft.sourceOwner !== currentOwner.current || active.current)
      return;
    if (!canInvite) {
      setError(PRIVATE_CHANNEL_ADD_DENIED_MESSAGE);
      return;
    }
    const attempt = new AbortController();
    active.current = { controller: attempt, draft }; // synchronous double-click guard
    setIsPending(true);
    setError(null);
    const isCurrent = () =>
      active.current?.controller === attempt &&
      !attempt.signal.aborted &&
      currentOwner.current === draft.sourceOwner;
    void (async () => {
      const mentionPubkeys = uniqueNormalizedPubkeys(
        await revalidate(
          [...draft.mentionPubkeys, ...draft.nonMemberPubkeys],
          draft.capturedChannelId,
          mentionRevalidationOptions(draft, "prepare"),
        ),
      );
      if (!isCurrent()) return;
      const admitted = new Set(mentionPubkeys);
      const originalNonMembers = new Set(
        draft.nonMemberPubkeys.map(normalizePubkey),
      );
      const nonMembers = [...originalNonMembers].filter((key) =>
        admitted.has(key),
      );
      const outgoingTags = (draft.outgoingTags ?? []).filter(
        (tag) =>
          tag[0] !== MENTION_REFERENCE_TAG ||
          !originalNonMembers.has(normalizePubkey(tag[1] ?? "")),
      );
      const managed = await getManagedAgentsByPubkey().catch(
        () => new Map<string, ManagedAgent>(),
      );
      if (!isCurrent()) return;
      const errors: string[] = [];
      for (const role of ["member", "bot"] as const) {
        const pubkeys = nonMembers.filter(
          (key) => !managed.has(key) && isAgentPubkey(key) === (role === "bot"),
        );
        if (pubkeys.length === 0) continue;
        const result = await addMembers({
          channelId: draft.capturedChannelId ?? undefined,
          pubkeys,
          role,
        });
        // An accepted add cannot be undone, but it never revives cancelled intent.
        if (!isCurrent()) return;
        errors.push(...result.errors.map((error) => error.error));
      }
      if (errors.length > 0) {
        setError(errors.join("; "));
        return;
      }
      if (!isCurrent()) return;
      await completeSend(
        {
          ...draft,
          mentionPubkeys,
          outgoingTags,
          invitationSignal: attempt.signal,
        },
        mentionPubkeys,
        outgoingTags,
      );
    })()
      .catch((error) => {
        if (isCurrent())
          setError(
            error instanceof Error
              ? error.message
              : "Could not invite members.",
          );
      })
      .finally(() => {
        if (active.current?.controller === attempt) {
          active.current = null;
          setIsPending(false);
        }
      });
  }, [
    draft,
    canInvite,
    revalidate,
    getManagedAgentsByPubkey,
    isAgentPubkey,
    addMembers,
    completeSend,
    setError,
  ]);
  return { invite, cancel, isPending };
}
