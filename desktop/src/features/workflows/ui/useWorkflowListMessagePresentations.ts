import { useQuery } from "@tanstack/react-query";

import { getEventsByIds } from "@/shared/api/tauri";
import type { RelayEvent, Workflow } from "@/shared/api/types";
import { getWorkflowTriggerConfig } from "./workflowDefinition";
import {
  type WorkflowTriggerPresentation,
  workflowTriggerMessageId,
} from "./useWorkflowTriggerPresentation";
import { validatedWorkflowMessageCandidate } from "./workflowMessageCandidates";

export type WorkflowMessagePresentation = Pick<
  WorkflowTriggerPresentation,
  "messageId" | "messageLabel" | "messageLoading"
>;

type WorkflowMessageLookup = {
  channelId: string;
  messageId: string;
  workflowId: string;
};

export function workflowMessageLookups(
  workflows: readonly Workflow[],
): WorkflowMessageLookup[] {
  return workflows.flatMap((workflow) => {
    const trigger = getWorkflowTriggerConfig(workflow.definition);
    const messageId = trigger ? workflowTriggerMessageId(trigger) : null;
    return workflow.channelId && messageId
      ? [{ channelId: workflow.channelId, messageId, workflowId: workflow.id }]
      : [];
  });
}

export async function loadWorkflowMessagePresentations(
  lookups: readonly WorkflowMessageLookup[],
  fetchEvents: (eventIds: string[]) => Promise<RelayEvent[]> = getEventsByIds,
): Promise<Map<string, WorkflowMessagePresentation>> {
  const eventIds = [...new Set(lookups.map(({ messageId }) => messageId))];
  if (eventIds.length === 0) return new Map();

  const events = await fetchEvents(eventIds);
  const eventById = new Map(
    events.map((event) => [event.id.toLowerCase(), event]),
  );
  return new Map(
    lookups.map(({ channelId, messageId, workflowId }) => {
      const message = validatedWorkflowMessageCandidate(
        eventById.get(messageId),
        {
          channelId,
          requestedId: messageId,
        },
      );
      return [
        workflowId,
        {
          messageId,
          messageLabel: message?.content?.trim() || null,
          messageLoading: false,
        },
      ];
    }),
  );
}

export function useWorkflowListMessagePresentations(
  workflows: readonly Workflow[],
): Map<string, WorkflowMessagePresentation> {
  const lookups = workflowMessageLookups(workflows);
  const lookupKey = lookups
    .map(({ channelId, messageId, workflowId }) =>
      [workflowId, channelId, messageId].join(":"),
    )
    .sort()
    .join(",");
  const query = useQuery({
    queryKey: ["workflow-list-message-presentations", lookupKey],
    queryFn: () => loadWorkflowMessagePresentations(lookups),
    enabled: lookups.length > 0,
    retry: false,
    staleTime: 60_000,
  });

  if (!query.isPending) return query.data ?? new Map();
  return new Map(
    lookups.map(({ messageId, workflowId }) => [
      workflowId,
      { messageId, messageLabel: null, messageLoading: true },
    ]),
  );
}
