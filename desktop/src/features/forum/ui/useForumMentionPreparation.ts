import * as React from "react";
import { useAddChannelMembersMutation } from "@/features/channels/hooks";
import { PRIVATE_CHANNEL_ADD_DENIED_MESSAGE } from "@/features/channels/lib/channelMemberAdmission";
import { useCanAddChannelMembers } from "@/features/channels/useCanAddChannelMembers";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { ChannelType } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

type PendingInvite = {
  channelId: string;
  pubkeys: string[];
  nonMemberPubkeys: string[];
  intendedAgentPubkeys: string[];
  resolve: (invited: boolean) => void;
};

/** Adapt the normal mention Invite dialog and authorized add to standalone forums. */
export function useForumMentionPreparation(
  channelId: string | null,
  channelType: ChannelType | null | undefined,
  mentions: UseMentionsResult,
) {
  const addMembers = useAddChannelMembersMutation(channelId);
  const canInvite = useCanAddChannelMembers(channelId);
  const [pending, setPending] = React.useState<PendingInvite | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [isInviting, setIsInviting] = React.useState(false);
  const pendingRef = React.useRef<PendingInvite | null>(null);
  const invitingRef = React.useRef<PendingInvite | null>(null);
  const attemptRef = React.useRef(0);
  const activeChannelRef = React.useRef(channelId);
  activeChannelRef.current = channelId;
  const mountedRef = React.useRef(false);

  const dismiss = React.useCallback(() => {
    const draft = pendingRef.current;
    attemptRef.current += 1;
    invitingRef.current = null;
    setIsInviting(false);
    pendingRef.current = null;
    setPending(null);
    setError(null);
    draft?.resolve(false);
  }, []);

  React.useLayoutEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      attemptRef.current += 1;
      pendingRef.current?.resolve(false);
      pendingRef.current = null;
    };
  }, []);
  React.useEffect(() => {
    if (pendingRef.current?.channelId !== channelId) dismiss();
  }, [channelId, dismiss]);

  const prepareMentionPubkeys = React.useCallback(
    async (pubkeys: string[], content: string) => {
      const attempt = ++attemptRef.current;
      const capturedChannelId = channelId;
      const isCurrent = () =>
        mountedRef.current &&
        activeChannelRef.current === capturedChannelId &&
        attemptRef.current === attempt;
      const intendedAgentPubkeys = [
        ...pubkeys.filter(mentions.isAgentPubkey),
        ...mentions
          .getDraftMentionRefs(content)
          .filter((ref) => ref.isAgent)
          .map((ref) => ref.pubkey),
      ];
      const agentPubkeys = new Set(intendedAgentPubkeys.map(normalizePubkey));
      // Local managed-agent lifecycle and channel-less/notes surfaces are not
      // part of this adapter. Relay-only agents use the same bot add as chat.
      const nonMemberPubkeys =
        capturedChannelId &&
        channelType === "forum" &&
        mentions.hasResolvedMembers
          ? [...new Set(pubkeys.map(normalizePubkey))].filter(
              (pubkey) =>
                agentPubkeys.has(pubkey) &&
                !mentions.isManagedAgentPubkey(pubkey) &&
                !mentions.memberPubkeys.has(pubkey),
            )
          : [];
      if (capturedChannelId && nonMemberPubkeys.length > 0) {
        const invited = await new Promise<boolean>((resolve) => {
          const draft = {
            channelId: capturedChannelId,
            pubkeys,
            nonMemberPubkeys,
            intendedAgentPubkeys,
            resolve,
          };
          pendingRef.current = draft;
          setError(null);
          setPending(draft);
        });
        if (!invited) return null;
      }
      if (!isCurrent()) return null;
      // The add mutation awaits membership invalidation. Publication still
      // requires a fresh authoritative directory/membership/policy read.
      try {
        const validated = await mentions.revalidateMentionPubkeys(
          pubkeys,
          capturedChannelId,
          { phase: "publish", intendedAgentPubkeys },
        );
        return isCurrent() ? validated : null;
      } catch (failure) {
        if (!isCurrent()) return null;
        throw failure;
      }
    },
    [channelId, channelType, mentions],
  );

  const invite = React.useCallback(async () => {
    const draft = pendingRef.current;
    if (!draft || invitingRef.current) return;
    if (!canInvite) {
      setError(PRIVATE_CHANNEL_ADD_DENIED_MESSAGE);
      return;
    }
    const isCurrent = () =>
      mountedRef.current &&
      activeChannelRef.current === draft.channelId &&
      pendingRef.current === draft;
    invitingRef.current = draft;
    setIsInviting(true);
    setError(null);
    try {
      // Preparation admits eligible owned nonmembers, not arbitrary targets.
      // Never use publication's membership gate before the authorized add.
      await mentions.revalidateMentionPubkeys(draft.pubkeys, draft.channelId, {
        phase: "prepare",
        intendedAgentPubkeys: draft.intendedAgentPubkeys,
      });
      if (!isCurrent()) return;
      const result = await addMembers.mutateAsync({
        channelId: draft.channelId,
        pubkeys: draft.nonMemberPubkeys,
        role: "bot",
      });
      if (!isCurrent()) return;
      if (result.errors.length > 0) {
        setError(result.errors.map((failure) => failure.error).join("; "));
        return;
      }
      pendingRef.current = null;
      setPending(null);
      draft.resolve(true);
    } catch (failure) {
      if (isCurrent())
        setError(
          failure instanceof Error
            ? failure.message
            : "Could not invite members.",
        );
    } finally {
      if (invitingRef.current === draft) {
        invitingRef.current = null;
        if (mountedRef.current) setIsInviting(false);
      }
    }
  }, [addMembers.mutateAsync, canInvite, mentions.revalidateMentionPubkeys]);

  return {
    prepareMentionPubkeys,
    nonMemberPromptProps: {
      canInvite,
      error,
      isInvitePending: isInviting,
      names: (pending?.nonMemberPubkeys ?? []).map(
        (pubkey) =>
          mentions.getMentionDisplayName(pubkey) ?? truncatePubkey(pubkey),
      ),
      onDismiss: dismiss,
      onInvite: () => void invite(),
      open: pending !== null,
    },
  };
}
