import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  channelsQueryKey,
  upsertCachedChannel,
  useCreateChannelMutation,
} from "@/features/channels/hooks";
import { useApplyTemplate } from "@/features/channel-templates/useApplyTemplate";
import { type Project, projectsQueryKey } from "@/features/projects/hooks";
import { isUnsupportedProjectKindError } from "@/features/projects/projectCreation";
import { addRelatedChannelToProject } from "@/features/projects/projectModels";
import {
  ProjectRevisionPublicationError,
  publishProjectRelatedChannelRevision,
} from "@/features/projects/projectRelatedChannelRevision";
import { markProjectDataAuthoritative } from "@/features/projects/projectSnapshot";
import { getProjectRevisionHeads } from "@/shared/api/projectRevisions";
import { relayClient } from "@/shared/api/relayClient";
import { deleteChannel as deleteChannelApi } from "@/shared/api/tauriChannels";
import type {
  Channel,
  ChannelVisibility,
  RelayEvent,
} from "@/shared/api/types";
import { KIND_PROJECT_ANNOUNCEMENT } from "@/shared/constants/kinds";

export type AddProjectChannelInput = {
  description?: string;
  name: string;
  ownerControlAgentPubkey?: string;
  project: Project;
  signAsManagedOwner?: boolean;
  templateId?: string;
  ttlSeconds?: number;
  visibility: ChannelVisibility;
};

export async function addProjectChannel(
  input: AddProjectChannelInput,
  deps: {
    applyAgents: (
      templateId: string | undefined,
      channelId: string,
    ) => void | Promise<void>;
    applyCanvas: (
      templateId: string | undefined,
      channelId: string,
      channelName: string,
    ) => Promise<void>;
    createChannel: ReturnType<typeof useCreateChannelMutation>["mutateAsync"];
    deleteChannel?: typeof deleteChannelApi;
    fetchEvents?: typeof relayClient.fetchEvents;
    fetchRevisionHeads?: typeof getProjectRevisionHeads;
    publishRevision?: typeof publishProjectRelatedChannelRevision;
  },
): Promise<{ channel: Channel; project: Project }> {
  const {
    applyAgents,
    applyCanvas,
    createChannel,
    deleteChannel = deleteChannelApi,
    fetchEvents = relayClient.fetchEvents.bind(relayClient),
    fetchRevisionHeads = getProjectRevisionHeads,
    publishRevision = publishProjectRelatedChannelRevision,
  } = deps;
  const targetOwner = input.project.owner.toLowerCase();

  const fetchLiveHead = async () =>
    (
      await fetchEvents({
        authors: [targetOwner],
        kinds: [KIND_PROJECT_ANNOUNCEMENT],
        "#d": [input.project.dtag],
        limit: 1,
      })
    )[0];
  const liveHead = await fetchLiveHead();
  if (!liveHead) {
    throw new Error(
      "Could not find this project on the relay. Refresh and try again.",
    );
  }
  if (liveHead.created_at > input.project.createdAt) {
    throw new Error(
      "This project was updated by another session while you were working. Refresh and try again.",
    );
  }

  const channel = await createChannel({
    channelType: "stream",
    description: input.description,
    name: input.name,
    ttlSeconds: input.ttlSeconds,
    visibility: input.visibility,
  });

  const removeCreatedChannelAndThrow = async (error: Error): Promise<never> => {
    try {
      await deleteChannel(channel.id);
    } catch (cleanupError) {
      throw new AggregateError(
        [error, cleanupError],
        "Buzz could not verify the project after creating the channel, and could not remove the unlinked channel. Delete it manually, then refresh and try again.",
      );
    }
    throw error;
  };

  const confirmedLiveHead = await fetchLiveHead().catch((error: unknown) =>
    removeCreatedChannelAndThrow(
      error instanceof Error
        ? error
        : new Error("Could not verify the project after creating the channel."),
    ),
  );
  if (!confirmedLiveHead || confirmedLiveHead.id !== liveHead.id) {
    await removeCreatedChannelAndThrow(
      new Error(
        "This project was updated by another session while the channel was being created. Refresh and try again.",
      ),
    );
  }

  const controlledSigner =
    input.ownerControlAgentPubkey || input.signAsManagedOwner
      ? {
          ownerControlAgentPubkey: input.ownerControlAgentPubkey,
          signAsManagedOwner: input.signAsManagedOwner,
        }
      : undefined;
  const revisionPromise = controlledSigner
    ? publishRevision(
        input.project,
        channel.id,
        "add-related-channel",
        undefined,
        controlledSigner,
      )
    : publishRevision(input.project, channel.id, "add-related-channel");
  let revision: RelayEvent | undefined;
  try {
    revision = await revisionPromise;
  } catch (error) {
    if (error instanceof ProjectRevisionPublicationError) {
      let storedRevision: RelayEvent | undefined;
      try {
        [storedRevision] = await fetchRevisionHeads([
          input.project.projectAddress,
        ]);
      } catch (reconciliationError) {
        throw new AggregateError(
          [error, reconciliationError],
          "Buzz could not confirm whether the Project channel change was accepted. The new channel was kept; refresh the Project before retrying.",
        );
      }
      const currentHeadIncludesChannel = storedRevision?.tags.some(
        (tag) =>
          tag[0] === "buzz-related-channel" &&
          tag[1]?.toLowerCase() === channel.id.toLowerCase(),
      );
      if (
        storedRevision &&
        (storedRevision.id.toLowerCase() === error.event.id.toLowerCase() ||
          currentHeadIncludesChannel)
      ) {
        revision = storedRevision;
      } else {
        await removeCreatedChannelAndThrow(
          isUnsupportedProjectKindError(error)
            ? new Error(
                "This relay does not support collaborative Project channels yet, so the channel could not be linked.",
              )
            : error,
        );
      }
    } else {
      await removeCreatedChannelAndThrow(
        isUnsupportedProjectKindError(error)
          ? new Error(
              "This relay does not support collaborative Project channels yet, so the channel could not be linked.",
            )
          : error instanceof Error
            ? error
            : new Error("Could not link the channel to this project."),
      );
    }
  }
  if (!revision) {
    throw new Error("Could not reconcile the Project channel change.");
  }

  await applyCanvas(input.templateId, channel.id, input.name);
  void applyAgents(input.templateId, channel.id);

  return {
    channel,
    project: {
      ...addRelatedChannelToProject(
        input.project,
        channel.id,
        revision.created_at,
      ),
      effectiveRevisionId: revision.id.toLowerCase(),
    },
  };
}

export function useAddProjectChannelMutation() {
  const queryClient = useQueryClient();
  const createChannelMutation = useCreateChannelMutation();
  const { applyAgents, applyCanvas } = useApplyTemplate();

  return useMutation({
    mutationFn: (input: AddProjectChannelInput) =>
      addProjectChannel(input, {
        applyAgents,
        applyCanvas,
        createChannel: createChannelMutation.mutateAsync,
      }),
    onSuccess: ({ channel, project }) => {
      markProjectDataAuthoritative(project, "local-write");
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.map((candidate) =>
          candidate.id === project.id ? project : candidate,
        ),
      );
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, channel),
      );
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
      void queryClient.invalidateQueries({
        queryKey: channelsQueryKey,
        refetchType: "none",
      });
    },
  });
}
