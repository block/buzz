import * as React from "react";
import { toast } from "sonner";

import { classifyAgentManagementOrigin } from "@/features/agents/agentManagementBuffer";
import { subscribeProjectChannelRequests } from "@/features/agents/observerRelayStore";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { useProjectsQuery } from "@/features/projects/hooks";
import type { ProjectChannelRequest } from "@/features/projects/projectChannelRequest";
import {
  advanceProjectChannelRequestQueue,
  createProjectChannelRequestQueue,
  enqueueProjectChannelRequest,
  type AcceptedProjectChannelRequest,
} from "@/features/projects/projectChannelRequestQueue";
import { useAddProjectChannelMutation } from "@/features/projects/useAddProjectChannel";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function useProjectChannelRequests() {
  const projectsQuery = useProjectsQuery();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const templatesQuery = useChannelTemplatesQuery();
  const mutation = useAddProjectChannelMutation();
  const [request, setRequest] = React.useState<ProjectChannelRequest | null>(
    null,
  );
  const [sourceAgentPubkey, setSourceAgentPubkey] = React.useState<
    string | null
  >(null);
  const [error, setError] = React.useState<string | null>(null);
  const acceptedRequests = React.useRef(createProjectChannelRequestQueue());
  const bufferedRequests = React.useRef<
    Array<{ agentPubkey: string; request: ProjectChannelRequest }>
  >([]);
  const managedAgentsRef = React.useRef(managedAgentsQuery.data);
  const channelsRef = React.useRef(channelsQuery.data);

  const accept = React.useEffectEvent(
    (agentPubkey: string, next: ProjectChannelRequest) => {
      if (
        classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          agentPubkey,
          next.request.homeChannelId,
        ) !== "accept"
      ) {
        return;
      }
      const result = enqueueProjectChannelRequest(acceptedRequests.current, {
        agentPubkey,
        request: next,
      });
      if (result.status === "show") showRequest(result.candidate);
    },
  );

  React.useEffect(() => {
    managedAgentsRef.current = managedAgentsQuery.data;
    channelsRef.current = channelsQuery.data;
    if (!managedAgentsQuery.data || !channelsQuery.data) return;
    const buffered = bufferedRequests.current.splice(0);
    for (const candidate of buffered) {
      accept(candidate.agentPubkey, candidate.request);
    }
  }, [channelsQuery.data, managedAgentsQuery.data]);

  React.useEffect(
    () =>
      subscribeProjectChannelRequests((agentPubkey, next) => {
        const origin = classifyAgentManagementOrigin(
          managedAgentsRef.current,
          channelsRef.current,
          agentPubkey,
          next.request.homeChannelId,
        );
        if (origin === "buffer") {
          bufferedRequests.current.push({ agentPubkey, request: next });
          if (bufferedRequests.current.length > 100) {
            bufferedRequests.current.shift();
          }
          return;
        }
        accept(agentPubkey, next);
      }),
    [],
  );

  function showRequest(candidate: AcceptedProjectChannelRequest | null) {
    setError(null);
    setSourceAgentPubkey(candidate?.agentPubkey ?? null);
    setRequest(candidate?.request ?? null);
  }

  function advanceRequestQueue() {
    showRequest(advanceProjectChannelRequestQueue(acceptedRequests.current));
  }

  const project =
    request == null
      ? null
      : ([...(projectsQuery.data ?? [])]
          .filter(
            (candidate) =>
              !candidate.legacy &&
              candidate.visibility !== "unlisted" &&
              candidate.projectChannelId === request.request.homeChannelId,
          )
          .sort((left, right) => left.createdAt - right.createdAt)[0] ?? null);
  const template =
    request?.request.templateName == null
      ? null
      : ((templatesQuery.data ?? []).find(
          (candidate) =>
            candidate.name.trim().toLocaleLowerCase() ===
            request.request.templateName?.trim().toLocaleLowerCase(),
        ) ?? null);

  function dismiss() {
    if (mutation.isPending) return;
    advanceRequestQueue();
  }

  async function approve() {
    if (!request || !sourceAgentPubkey || !project) {
      setError(
        "The project home could not be resolved. Refresh and try again.",
      );
      return;
    }
    const identity = identityQuery.data?.pubkey;
    const owner = normalizePubkey(project.owner);
    const agent = normalizePubkey(sourceAgentPubkey);
    if ((!identity || normalizePubkey(identity) !== owner) && agent !== owner) {
      setError("Only the project owner can approve this channel.");
      return;
    }
    if (request.request.templateName && !template) {
      setError(
        `Channel template "${request.request.templateName}" is not available on this device.`,
      );
      return;
    }
    setError(null);
    try {
      await mutation.mutateAsync({
        description: request.request.description,
        name: request.request.name,
        ownerControlAgentPubkey:
          agent === owner ? sourceAgentPubkey : undefined,
        project,
        templateId: template?.id,
        ttlSeconds: request.request.ttlSeconds,
        visibility: request.request.visibility,
      });
      toast.success(`Channel "#${request.request.name}" created.`);
      advanceRequestQueue();
    } catch (nextError) {
      setError(
        nextError instanceof Error
          ? nextError.message
          : "Failed to create the project channel.",
      );
    }
  }

  return {
    approve,
    dismiss,
    error,
    isPending: mutation.isPending,
    project,
    request,
    template,
  };
}
