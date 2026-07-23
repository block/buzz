import {
  KIND_STREAM_MESSAGE,
  KIND_TEXT_NOTE,
} from "../../shared/constants/kinds.ts";

/**
 * Issue/PR comments without agent mentions stay kind:1 (relay has no NIP-22
 * kind 1111). Mentions must be kind:9 + channel `h` so buzz-acp's per-channel
 * Mentions subscription (#h + #p + kinds [9,…]) can deliver them — see #2462.
 */
export function projectCommentKindAndChannelTags(project, mentionPubkeys) {
  if (mentionPubkeys.length === 0) {
    return { kind: KIND_TEXT_NOTE, channelTags: [] };
  }
  const channelId =
    typeof project.projectChannelId === "string"
      ? project.projectChannelId.trim()
      : "";
  if (!channelId) {
    throw new Error(
      "This project has no discussion channel. Link a channel before @mentioning agents, or mention them in a channel instead.",
    );
  }
  return {
    kind: KIND_STREAM_MESSAGE,
    channelTags: [["h", channelId]],
  };
}
