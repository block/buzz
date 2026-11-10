import type { RelayEvent } from "@/shared/api/types";
import { KIND_PROJECT_ANNOUNCEMENT } from "@/shared/constants/kinds";
import {
  isValidProjectChannelId,
  MAX_PROJECT_RELATED_CHANNELS,
  PROJECT_RELATED_CHANNEL_TAG,
  validateProjectEventEnvelope,
} from "@/features/projects/projectModels";
import type { ProjectEventTemplate } from "./projectCreation";

/**
 * Appends a `buzz-related-channel` tag to a live project head. Every other
 * tag is preserved so adding a stream cannot erase unknown metadata.
 */
export function buildProjectRelatedChannelPatchTemplate({
  channelId,
  liveHead,
  ownerPubkey,
}: {
  channelId: string;
  liveHead: RelayEvent;
  ownerPubkey: string;
}): { alreadyBound: boolean; project: ProjectEventTemplate } {
  const normalizedOwner = ownerPubkey.trim().toLowerCase();
  if (normalizedOwner !== liveHead.pubkey.toLowerCase()) {
    throw new Error("Only the project owner can add channels.");
  }
  const normalizedChannelId = channelId.trim();
  if (!isValidProjectChannelId(normalizedChannelId)) {
    throw new Error("Project channel is invalid.");
  }
  const homeChannelId = liveHead.tags.find(
    (tag) => tag[0] === "buzz-channel",
  )?.[1];
  if (homeChannelId === normalizedChannelId) {
    throw new Error("That channel is already this project's home.");
  }
  const existingRelated = liveHead.tags
    .filter((tag) => tag[0] === PROJECT_RELATED_CHANNEL_TAG)
    .map((tag) => tag[1])
    .filter((value): value is string => Boolean(value));
  if (existingRelated.includes(normalizedChannelId)) {
    validateProjectEventEnvelope(liveHead.tags, liveHead.content);
    return {
      alreadyBound: true,
      project: {
        kind: KIND_PROJECT_ANNOUNCEMENT,
        content: liveHead.content,
        tags: liveHead.tags.map((tag) => [...tag]),
      },
    };
  }
  if (existingRelated.length >= MAX_PROJECT_RELATED_CHANNELS) {
    throw new Error(
      `A project cannot contain more than ${MAX_PROJECT_RELATED_CHANNELS} extra channels.`,
    );
  }
  const tags = [
    ...liveHead.tags.map((tag) => [...tag]),
    [PROJECT_RELATED_CHANNEL_TAG, normalizedChannelId],
  ];
  validateProjectEventEnvelope(tags, liveHead.content);
  return {
    alreadyBound: false,
    project: {
      kind: KIND_PROJECT_ANNOUNCEMENT,
      content: liveHead.content,
      tags,
    },
  };
}
