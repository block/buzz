import { relayClient } from "@/shared/api/relayClient";
import { getChannelMembers, signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { KIND_STREAM_MESSAGE, KIND_TEXT_NOTE } from "@/shared/constants/kinds";

import type { ProjectIssue } from "./projectIssues.mjs";
import { nextProjectIssueCommentCreatedAt } from "./projectIssues.mjs";
import type { Repository } from "./projectModels";

export async function createProjectIssueComment({
  agentMentionPubkeys = [],
  content,
  mediaTags,
  mentionPubkeys = [],
  issue,
  project,
}: {
  agentMentionPubkeys?: string[];
  content: string;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
  issue: ProjectIssue;
  project: Repository;
}): Promise<void> {
  const body = content.trim();
  if (!body) throw new Error("Comment cannot be empty.");

  const identity = await getIdentity();
  let kind = KIND_TEXT_NOTE;
  let channelTags: string[][] = [];
  const authorizedAgentPubkeys = new Set<string>();
  if (agentMentionPubkeys.length > 0) {
    if (!project.channelId) {
      throw new Error(
        "Agent mentions require this repository to be bound to a discussion channel.",
      );
    }
    const channelMembers = await getChannelMembers(project.channelId);
    const memberPubkeys = new Set(
      channelMembers.map((member) => member.pubkey.toLowerCase()),
    );
    for (const member of channelMembers) {
      if (member.role === "bot" || member.isAgent) {
        authorizedAgentPubkeys.add(member.pubkey.toLowerCase());
      }
    }
    if (!memberPubkeys.has(identity.pubkey.toLowerCase())) {
      throw new Error(
        "Only repository channel members can address agents from an issue.",
      );
    }
    if (
      agentMentionPubkeys.some(
        (pubkey) => !memberPubkeys.has(pubkey.toLowerCase()),
      )
    ) {
      throw new Error(
        "Addressed agents must be members of the repository channel.",
      );
    }
    kind = KIND_STREAM_MESSAGE;
    channelTags = [["h", project.channelId]];
  }

  const recipients = new Set([
    project.owner.toLowerCase(),
    issue.author.toLowerCase(),
    ...issue.recipients.map((recipient) => recipient.toLowerCase()),
    ...mentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
  ]);
  if (kind === KIND_STREAM_MESSAGE) {
    const addressedAgents = new Set(
      agentMentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
    );
    for (const recipient of recipients) {
      if (
        authorizedAgentPubkeys.has(recipient) &&
        !addressedAgents.has(recipient)
      ) {
        recipients.delete(recipient);
      }
    }
  }
  const event = await signRelayEvent({
    kind,
    content: body,
    createdAt: nextProjectIssueCommentCreatedAt(
      issue,
      Math.floor(Date.now() / 1_000),
      identity.pubkey,
    ),
    tags: [
      ...channelTags,
      ["e", issue.id, "", "root"],
      ["a", project.repoAddress],
      ...[...recipients].map((recipient) => ["p", recipient]),
      ...(mediaTags ?? []),
    ],
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting issue comment.",
    "Failed to post issue comment.",
  );
}
