import * as React from "react";
import { useMutation } from "@tanstack/react-query";
import { authorizeExternalAgent } from "@/features/channels/api/externalAgentAuthorization";
import type { ChannelMember } from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type AuthorizationTarget = {
  label: string;
  member: ChannelMember;
};

export function useExternalAgentAuthorization(channelId: string | null) {
  const [target, setTarget] = React.useState<AuthorizationTarget | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const mutation = useMutation({
    mutationFn: async (authorizationTarget: AuthorizationTarget) => {
      if (!channelId) throw new Error("No channel selected.");
      const authTag = await authorizeExternalAgent(
        channelId,
        authorizationTarget.member.pubkey,
      );
      await writeTextToClipboard(authTag);
      return authorizationTarget;
    },
    onSuccess: (authorizedTarget) => {
      setTarget(null);
      setNotice(
        `${authorizedTarget.label} authorized. Add the copied value to the agent as BUZZ_AUTH_TAG.`,
      );
    },
  });
  const error = mutation.error instanceof Error ? mutation.error.message : null;

  useFeedbackToasts(notice, error);

  return {
    error: mutation.error,
    isPending: mutation.isPending,
    onConfirm: () => {
      if (target) mutation.mutate(target);
    },
    onOpenChange: (open: boolean) => {
      if (!open && !mutation.isPending) setTarget(null);
    },
    open: (authorizationTarget: AuthorizationTarget) => {
      mutation.reset();
      setNotice(null);
      setTarget(authorizationTarget);
    },
    target,
  };
}
