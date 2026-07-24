import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_GROUP_ADD_MEMBER,
  KIND_GROUP_CREATE,
  KIND_GROUP_DELETE,
  KIND_GROUP_EDIT,
  KIND_GROUP_REMOVE_MEMBER,
  KIND_GROUP_STATE,
} from "@/shared/constants/kinds";

export type UserGroup = {
  id: string;
  handle: string;
  name: string;
  description: string;
  creator: string;
  memberPubkeys: string[];
  defaultChannelIds: string[];
};

export type CreateUserGroupInput = UserGroup;

export type UpdateUserGroupInput = {
  group: UserGroup;
  next: UserGroup;
};

export type UserGroupErrorCode = "conflict" | "forbidden" | "rejected";

export class UserGroupError extends Error {
  readonly code: UserGroupErrorCode;

  constructor(message: string, code: UserGroupErrorCode) {
    super(message);
    this.code = code;
    this.name = "UserGroupError";
  }
}

function getTag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

function getTags(event: RelayEvent, name: string): string[] {
  return event.tags
    .filter((tag) => tag[0] === name && tag[1])
    .map((tag) => tag[1]);
}

export function parseUserGroupSnapshot(event: RelayEvent): UserGroup | null {
  if (
    event.kind !== KIND_GROUP_STATE ||
    event.tags.some((tag) => tag[0] === "deleted")
  ) {
    return null;
  }

  const id = getTag(event, "d")?.trim();
  const handle = getTag(event, "handle")?.trim();
  const name = getTag(event, "name")?.trim();
  const creator = getTag(event, "creator")?.trim().toLowerCase();
  if (!id || !handle || !name || !creator) {
    return null;
  }

  return {
    id,
    handle,
    name,
    description: getTag(event, "description") ?? "",
    creator,
    memberPubkeys: [...new Set(getTags(event, "p").map(normalizePubkey))],
    defaultChannelIds: [...new Set(getTags(event, "channel"))],
  };
}

export function userGroupIdFromSnapshot(event: RelayEvent): string | null {
  return getTag(event, "d")?.trim() || null;
}

export function groupsFromSnapshotEvents(events: RelayEvent[]): UserGroup[] {
  const latestById = new Map<string, RelayEvent>();
  for (const event of events) {
    const id = userGroupIdFromSnapshot(event);
    if (!id) continue;
    const current = latestById.get(id);
    if (!current || event.created_at >= current.created_at) {
      latestById.set(id, event);
    }
  }

  return [...latestById.values()]
    .map(parseUserGroupSnapshot)
    .filter((group): group is UserGroup => group !== null)
    .sort((left, right) => left.name.localeCompare(right.name));
}

export async function listUserGroups(): Promise<UserGroup[]> {
  return groupsFromSnapshotEvents(
    await relayClient.fetchEvents({
      kinds: [KIND_GROUP_STATE],
      limit: 500,
    }),
  );
}

export async function subscribeToUserGroups(
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    { kinds: [KIND_GROUP_STATE], limit: 0 },
    onEvent,
  );
}

function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function mapGroupError(error: unknown): never {
  const message =
    error instanceof Error && error.message
      ? error.message
      : "Relay rejected the user-group change.";
  const lower = message.toLowerCase();
  const code: UserGroupErrorCode =
    lower.startsWith("duplicate:") || lower.includes("already exists")
      ? "conflict"
      : lower.startsWith("restricted:") ||
          lower.includes("forbidden") ||
          lower.includes("not authorized")
        ? "forbidden"
        : "rejected";
  throw new UserGroupError(message, code);
}

async function publishGroupCommand(
  kind: number,
  tags: string[][],
): Promise<void> {
  try {
    const event = await signRelayEvent({ kind, content: "", tags });
    const signedPubkeys = new Set(
      event.tags
        .filter((tag) => tag[0] === "p" && tag[1])
        .map((tag) => normalizePubkey(tag[1])),
    );
    const missingPubkey = tags
      .filter((tag) => tag[0] === "p" && tag[1])
      .map((tag) => normalizePubkey(tag[1]))
      .find((pubkey) => !signedPubkeys.has(pubkey));
    if (missingPubkey) {
      throw new UserGroupError(
        "The desktop signer could not preserve one selected member. Remove your own account from the group and try again.",
        "rejected",
      );
    }
    await relayClient.publishEvent(
      event,
      "Timed out while updating the user group.",
      "Failed to update the user group.",
    );
  } catch (error) {
    mapGroupError(error);
  }
}

export async function createUserGroup(
  input: CreateUserGroupInput,
): Promise<UserGroup> {
  const tags: string[][] = [
    ["g", input.id],
    ["handle", input.handle],
    ["name", input.name],
  ];
  if (input.description) tags.push(["description", input.description]);
  tags.push(
    ...input.memberPubkeys.map((pubkey) => ["p", normalizePubkey(pubkey)]),
    ...input.defaultChannelIds.map((channelId) => ["channel", channelId]),
  );
  await publishGroupCommand(KIND_GROUP_CREATE, tags);
  return input;
}

export async function updateUserGroup(
  input: UpdateUserGroupInput,
): Promise<UserGroup> {
  const { group, next } = input;
  const editTags: string[][] = [["g", group.id]];
  if (group.handle !== next.handle) editTags.push(["handle", next.handle]);
  if (group.name !== next.name) editTags.push(["name", next.name]);
  if (group.description !== next.description) {
    editTags.push(["description", next.description]);
  }
  if (!sameValues(group.defaultChannelIds, next.defaultChannelIds)) {
    editTags.push(
      ...(next.defaultChannelIds.length > 0
        ? next.defaultChannelIds.map((id) => ["channel", id])
        : [["channel", ""]]),
    );
  }
  if (editTags.length > 1) {
    await publishGroupCommand(KIND_GROUP_EDIT, editTags);
  }

  const previousMembers = new Set(group.memberPubkeys.map(normalizePubkey));
  const nextMembers = new Set(next.memberPubkeys.map(normalizePubkey));
  const added = [...nextMembers].filter(
    (pubkey) => !previousMembers.has(pubkey),
  );
  const removed = [...previousMembers].filter(
    (pubkey) => !nextMembers.has(pubkey),
  );
  if (added.length > 0) {
    await publishGroupCommand(KIND_GROUP_ADD_MEMBER, [
      ["g", group.id],
      ...added.map((pubkey) => ["p", pubkey]),
    ]);
  }
  if (removed.length > 0) {
    await publishGroupCommand(KIND_GROUP_REMOVE_MEMBER, [
      ["g", group.id],
      ...removed.map((pubkey) => ["p", pubkey]),
    ]);
  }
  return next;
}

export async function deleteUserGroup(id: string): Promise<string> {
  await publishGroupCommand(KIND_GROUP_DELETE, [["g", id]]);
  return id;
}

function sameValues(left: string[], right: string[]): boolean {
  if (left.length !== right.length) return false;
  const values = new Set(left);
  return right.every((value) => values.has(value));
}
