import { useQuery } from "@tanstack/react-query";

import { getEventById } from "@/shared/api/tauri";
import { useWorkflowAuthorPresentation } from "./useWorkflowAuthorPresentation";
import { parseConditionExpressions } from "./workflowConditionExpression";
import type { TriggerConfig } from "./workflowFormTypes";
import {
  normalizeMessageEventId,
  validatedWorkflowMessageCandidate,
} from "./workflowMessageCandidates";
import { workflowTriggerDescription } from "./workflowTriggerDescription";

export type WorkflowTriggerPresentation = ReturnType<
  typeof useWorkflowAuthorPresentation
> & {
  messageId: string | null;
  messageLabel: string | null;
  messageLoading: boolean;
};

export function workflowTriggerMessageId(
  trigger: TriggerConfig,
): string | null {
  const conditions = trigger.filter
    ? parseConditionExpressions(trigger.filter, trigger.on)
    : [];
  const condition = conditions?.find(
    ({ field }) => field === "trigger_message_id",
  );
  return condition ? normalizeMessageEventId(condition.value) : null;
}

export function useWorkflowTriggerPresentation(
  trigger: TriggerConfig,
  workflowChannelId?: string | null,
): WorkflowTriggerPresentation {
  const author = useWorkflowAuthorPresentation(trigger);
  const messageId = workflowTriggerMessageId(trigger);
  const messageQuery = useQuery({
    enabled: Boolean(messageId && workflowChannelId),
    queryKey: ["workflow-trigger-message", workflowChannelId, messageId],
    queryFn: () => getEventById(messageId ?? ""),
    retry: false,
    staleTime: 60_000,
  });
  const message =
    workflowChannelId && messageId
      ? validatedWorkflowMessageCandidate(messageQuery.data, {
          channelId: workflowChannelId,
          requestedId: messageId,
        })
      : null;
  const messageLoading = Boolean(
    messageId && workflowChannelId && messageQuery.isPending,
  );
  const messageLabel = message?.content?.trim() || null;

  return {
    ...author,
    description: workflowTriggerDescription(trigger, {
      authorLabel: author.label ?? undefined,
      authorLoading: author.loading,
      messageLabel: messageLabel ?? undefined,
      messageLoading,
    }),
    messageId,
    messageLabel,
    messageLoading,
  };
}
