import * as React from "react";
import { toast } from "sonner";
import { getMembershipAdmissionEpoch } from "@/features/channels/membershipDirectorySync";
import { canAddChannelMembers } from "@/features/channels/lib/channelMemberAdmission";
import { getChannelMembers, getChannels } from "@/shared/api/tauriChannels";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { MentionSuggestion } from "../ui/MentionAutocomplete";
import type { MentionRevalidationOptions } from "./agentMentionRevalidation";

/** Displayed rows belong to their search; default pins start a new identity action. */
export type MentionAdmissionOwner = "displayed-choice" | "default-agent";

type EditorPosition = { text: string; cursor: number };

/** A selection freezes intent, not authority. No local writes occur before commit. */
export function useMentionAdmission({
  generation,
  channelId,
  channelType,
  currentPubkey,
  agentPubkeys,
  revalidate,
  personaIds,
  isPending,
  validateSuggestion,
}: {
  generation: React.MutableRefObject<number>;
  isPending: () => boolean;
  validateSuggestion?: (suggestion: MentionSuggestion) => Promise<void>;
  channelId: string | null;
  channelType?: string | null;
  currentPubkey: string | null;
  agentPubkeys: ReadonlySet<string>;
  personaIds: ReadonlySet<string>;
  revalidate: (
    keys: readonly string[],
    channelId?: string | null,
    options?: MentionRevalidationOptions,
  ) => Promise<string[]>;
}) {
  const scope = `${channelId}:${channelType}:${currentPubkey}`;
  const scopeRef = React.useRef(scope);
  if (scopeRef.current !== scope) {
    scopeRef.current = scope;
    generation.current++;
  }
  const operation = React.useRef(0);
  const pendingCleanup = React.useRef<(() => void) | null>(null);
  React.useEffect(() => {
    const retire = () => {
      generation.current++;
      operation.current++;
      pendingCleanup.current?.();
      pendingCleanup.current = null;
    };
    window.addEventListener("blur", retire);
    return () => {
      retire();
      window.removeEventListener("blur", retire);
    };
  }, [generation]);
  const displayedRequest = generation.current;
  return React.useCallback(
    async (
      suggestion: MentionSuggestion,
      readEditor: () => EditorPosition,
      commit: () => void,
      owner: MentionAdmissionOwner = "displayed-choice",
    ): Promise<boolean> => {
      if (scope !== scopeRef.current) return false;
      if (
        owner === "displayed-choice" &&
        (displayedRequest !== generation.current || isPending())
      )
        return false;
      // Capture the current action, not a render preceding deliberate caret movement.
      // Both owners retain exactly the same async retirement and authority fences.
      const request = generation.current;
      pendingCleanup.current?.();
      const ticket = ++operation.current;
      const position = readEditor();
      const membershipEpoch = getMembershipAdmissionEpoch();
      let departed = false;
      const checkEditor = () => {
        const next = readEditor();
        if (next.text !== position.text || next.cursor !== position.cursor)
          departed = true;
      };
      const retire = () => {
        departed = true;
      };
      document.addEventListener("selectionchange", checkEditor);
      document.addEventListener("input", retire, true);
      document.addEventListener("focusout", retire, true);
      const cleanup = () => {
        departed = true;
        document.removeEventListener("selectionchange", checkEditor);
        document.removeEventListener("input", retire, true);
        document.removeEventListener("focusout", retire, true);
      };
      pendingCleanup.current = cleanup;
      try {
        const recipients = suggestion.teamMembers ?? [suggestion];
        const keys = recipients.flatMap((member) =>
          member.pubkey ? [normalizePubkey(member.pubkey)] : [],
        );
        const intendedAgents = keys.filter(
          (key) =>
            suggestion.isAgent ||
            agentPubkeys.has(key) ||
            suggestion.kind === "team",
        );
        for (const recipient of recipients) {
          if (recipient.personaId && !personaIds.has(recipient.personaId))
            throw new Error(
              "That persona is no longer available. Choose a mention again.",
            );
        }
        const humans = keys.filter((key) => !intendedAgents.includes(key));
        if (humans.length && channelType !== "dm") {
          if (!channelId)
            throw new Error("Choose a channel before mentioning this person.");
          const [members, directory] = await Promise.all([
            getChannelMembers(channelId),
            getChannels(null),
          ]);
          const channel = directory.channels?.find(
            (item) => item.id === channelId,
          );
          const memberKeys = new Set(
            members.map((member) => normalizePubkey(member.pubkey)),
          );
          const selfRole = members.find(
            (member) => normalizePubkey(member.pubkey) === currentPubkey,
          )?.role;
          if (
            humans.some((key) => !memberKeys.has(key)) &&
            !canAddChannelMembers({
              channelType: channel?.channelType,
              visibility: channel?.visibility,
              selfRole,
            })
          ) {
            throw new Error(
              "Channel access changed. Choose a mention again or remove it.",
            );
          }
        }
        await revalidate(keys, channelId, {
          phase: "prepare",
          intendedAgentPubkeys: intendedAgents,
        });
        // Definitions are checked after permission preparation: a deferred
        // exact-key permission response cannot vouch for old persona linkage.
        if (validateSuggestion) await validateSuggestion(suggestion);
        checkEditor();
        if (membershipEpoch !== getMembershipAdmissionEpoch())
          throw new Error("Channel access changed. Choose a mention again.");
        if (
          departed ||
          ticket !== operation.current ||
          request !== generation.current
        )
          return false;
        operation.current++;
        commit();
        return true;
      } catch (error) {
        if (
          !departed &&
          ticket === operation.current &&
          request === generation.current
        )
          toast.error(error instanceof Error ? error.message : String(error));
        return false;
      } finally {
        cleanup();
        if (pendingCleanup.current === cleanup) pendingCleanup.current = null;
      }
    },
    [
      displayedRequest,
      scope,
      generation,
      isPending,
      validateSuggestion,
      revalidate,
      channelId,
      channelType,
      currentPubkey,
      agentPubkeys,
      personaIds,
    ],
  );
}

/** Preserve the first displayed labels, exact recipients and order for a request. */
export function useStableMentionSuggestions(
  request: string,
  incoming: MentionSuggestion[],
) {
  const snapshot = React.useRef<{ request: string; rows: MentionSuggestion[] }>(
    { request, rows: [] },
  );
  if (snapshot.current.request !== request)
    snapshot.current = { request, rows: [] };
  const identity = (row: MentionSuggestion) =>
    `${row.kind}:${row.pubkey ?? row.personaId ?? row.teamId}`;
  const seen = new Set(snapshot.current.rows.map(identity));
  const added = incoming.filter((row) => !seen.has(identity(row)));
  if (added.length)
    snapshot.current.rows = [
      ...snapshot.current.rows,
      ...added
        .slice(0, Math.max(0, 50 - snapshot.current.rows.length))
        .map((row) => ({
          ...row,
          teamMembers: row.teamMembers?.map((member) => ({ ...member })),
        })),
    ];
  return snapshot.current.rows;
}
