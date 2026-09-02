import type { RelayEvent } from "@/shared/api/types";
import { KIND_PROJECT_RELATED_CHANNEL_SNAPSHOT } from "@/shared/constants/kinds";
import {
  isValidProjectChannelId,
  MAX_PROJECT_RELATED_CHANNELS,
} from "./projectModels";

const SNAPSHOT_D_PREFIX = "buzz:project-related-channels:v1";

export async function projectRelatedChannelSnapshotD(
  projectAddress: string,
): Promise<string> {
  const prefix = new TextEncoder().encode(SNAPSHOT_D_PREFIX);
  const address = new TextEncoder().encode(projectAddress);
  const bytes = new Uint8Array(prefix.length + 1 + address.length);
  bytes.set(prefix);
  bytes.set(address, prefix.length + 1);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (value) =>
    value.toString(16).padStart(2, "0"),
  ).join("");
}

/** Parse one trusted relay snapshot with its canonical deterministic envelope. */
export async function parseProjectRelatedChannelSnapshot(
  event: RelayEvent,
  projectAddress: string,
): Promise<string[] | null> {
  const expectedD = await projectRelatedChannelSnapshotD(projectAddress);
  if (
    event.kind !== KIND_PROJECT_RELATED_CHANNEL_SNAPSHOT ||
    event.content !== "" ||
    event.tags[0]?.length !== 2 ||
    event.tags[0]?.[0] !== "d" ||
    event.tags[0]?.[1] !== expectedD ||
    event.tags[1]?.length !== 2 ||
    event.tags[1]?.[0] !== "a" ||
    event.tags[1]?.[1] !== projectAddress
  ) {
    return null;
  }

  const channelIds: string[] = [];
  for (const tag of event.tags.slice(2)) {
    if (
      tag.length !== 2 ||
      tag[0] !== "c" ||
      !tag[1] ||
      !isValidProjectChannelId(tag[1]) ||
      tag[1] !== tag[1].toLowerCase() ||
      (channelIds.at(-1) ?? "") >= tag[1]
    ) {
      return null;
    }
    channelIds.push(tag[1]);
  }
  if (channelIds.length > MAX_PROJECT_RELATED_CHANNELS) return null;
  return channelIds;
}
