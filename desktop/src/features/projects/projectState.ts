import { verifyEvent } from "nostr-tools/pure";

import { getRelaySelf } from "@/features/moderation/lib/relaySelf";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { signRelayEvent } from "@/shared/api/tauri";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_PROJECT_CHANGE,
  KIND_PROJECT_STATE,
} from "@/shared/constants/kinds";
import type { Project } from "./projectModels";

const MAX_PROJECT_D_TAG_BYTES = 1_024;
const MAX_PATCH_CHANNELS = 64;
const MAX_REVISION = 9_223_372_036_854_775_807n;
const CANONICAL_UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const LOWER_HEX_64 = /^[0-9a-f]{64}$/;

export type ProjectState = {
  deleted: boolean;
  identityEventId: string;
  projectTags: string[][];
  revision: string;
};

export type ProjectRelatedChannelPatch = {
  add: string[];
  remove: string[];
};

export type ProjectRelatedChannelChangeTemplate = {
  content: string;
  kind: number;
  tags: string[][];
};

type ProjectStateFilter = {
  "#a": string[];
  authors: string[];
  kinds: number[];
  limit: number;
};

type ProjectStateDeps = {
  fetchEvents: (filter: ProjectStateFilter) => Promise<RelayEvent[]>;
  getRelayPubkey: () => Promise<string | null>;
};

type ChangeProjectRelatedChannelsDeps = ProjectStateDeps & {
  publishEvent: (
    event: RelayEvent,
    timeoutMessage: string,
    failureMessage: string,
  ) => Promise<unknown>;
  signEvent: (
    input: ProjectRelatedChannelChangeTemplate,
  ) => Promise<RelayEvent>;
};

function parseProjectCoordinate(coordinate: string): { dtag: string } {
  const first = coordinate.indexOf(":");
  const second = coordinate.indexOf(":", first + 1);
  const kind = coordinate.slice(0, first);
  const owner = coordinate.slice(first + 1, second);
  const dtag = coordinate.slice(second + 1);
  if (
    kind !== String(KIND_PROJECT_ANNOUNCEMENT) ||
    second < 0 ||
    !LOWER_HEX_64.test(owner) ||
    dtag.length === 0 ||
    new TextEncoder().encode(dtag).byteLength > MAX_PROJECT_D_TAG_BYTES
  ) {
    throw new Error("Invalid canonical Project coordinate.");
  }
  return { dtag };
}

function exactObjectKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
): boolean {
  const keys = Object.keys(value).sort();
  return (
    keys.length === expected.length &&
    keys.every((key, index) => key === [...expected].sort()[index])
  );
}

function parseStrictContent(content: string): {
  deleted: boolean;
  projectTags: string[][];
} {
  let parsed: unknown;
  try {
    parsed = JSON.parse(content);
  } catch {
    throw new Error("Project State content is not valid JSON.");
  }
  if (
    !parsed ||
    typeof parsed !== "object" ||
    Array.isArray(parsed) ||
    !exactObjectKeys(parsed as Record<string, unknown>, [
      "deleted",
      "project_tags",
      "v",
    ])
  ) {
    throw new Error("Project State content is not strict version 1.");
  }
  const body = parsed as Record<string, unknown>;
  if (
    body.v !== 1 ||
    typeof body.deleted !== "boolean" ||
    !Array.isArray(body.project_tags) ||
    !body.project_tags.every(
      (tag) =>
        Array.isArray(tag) &&
        tag.length > 0 &&
        tag.every((value) => typeof value === "string"),
    )
  ) {
    throw new Error("Project State content is not strict version 1.");
  }
  const projectTags = body.project_tags as string[][];
  if (body.deleted && projectTags.length !== 0) {
    throw new Error("Deleted Project State must not contain Project tags.");
  }
  return { deleted: body.deleted, projectTags };
}

function singleExactTag(event: RelayEvent, name: string): string[] | undefined {
  const tags = event.tags.filter((tag) => tag[0] === name);
  return tags.length === 1 ? tags[0] : undefined;
}

function hasValidSignature(event: RelayEvent): boolean {
  try {
    return verifyEvent(event);
  } catch {
    return false;
  }
}

/** Validate one relay-authored NIP-PC projection for an exact Project coordinate. */
export function parseProjectState(
  event: RelayEvent,
  relayPubkey: string,
  coordinate: string,
): ProjectState {
  const { dtag } = parseProjectCoordinate(coordinate);
  const normalizedRelay = relayPubkey.trim().toLowerCase();
  if (!LOWER_HEX_64.test(normalizedRelay)) {
    throw new Error("The relay did not advertise a valid signing identity.");
  }
  if (
    event.kind !== KIND_PROJECT_STATE ||
    event.pubkey !== normalizedRelay ||
    !hasValidSignature(event)
  ) {
    throw new Error("Project State is not validly signed by this relay.");
  }

  const coordinateTag = singleExactTag(event, "a");
  if (coordinateTag?.length !== 2 || coordinateTag[1] !== coordinate) {
    throw new Error("Project State does not match the requested coordinate.");
  }
  const revisionTag = singleExactTag(event, "rev");
  const revision = revisionTag?.[1] ?? "";
  if (
    revisionTag?.length !== 2 ||
    !/^[1-9][0-9]*$/.test(revision) ||
    BigInt(revision) > MAX_REVISION
  ) {
    throw new Error("Project State revision is not canonical.");
  }
  const identityTags = event.tags.filter(
    (tag) => tag[0] === "e" && tag[3] === "identity",
  );
  if (
    identityTags.length !== 1 ||
    identityTags[0].length !== 4 ||
    !LOWER_HEX_64.test(identityTags[0][1] ?? "") ||
    identityTags[0][2] !== ""
  ) {
    throw new Error("Project State identity marker is malformed.");
  }
  const { deleted, projectTags } = parseStrictContent(event.content);
  const projectDTags = projectTags.filter((tag) => tag[0] === "d");
  if (
    !deleted &&
    (projectDTags.length !== 1 ||
      projectDTags[0].length !== 2 ||
      projectDTags[0][1] !== dtag)
  ) {
    throw new Error(
      "Project State tags do not match the requested coordinate.",
    );
  }
  return {
    deleted,
    identityEventId: identityTags[0][1],
    projectTags,
    revision,
  };
}

function projectCoordinate(
  project: string | Pick<Project, "projectAddress">,
): string {
  return typeof project === "string" ? project : project.projectAddress;
}

/** Fetch and validate the current relay-authored state at mutation time. */
export async function fetchProjectState(
  project: string | Pick<Project, "projectAddress">,
  deps?: Partial<ProjectStateDeps>,
): Promise<ProjectState | null> {
  const coordinate = projectCoordinate(project);
  parseProjectCoordinate(coordinate);
  const getRelayPubkey = deps?.getRelayPubkey ?? getRelaySelf;
  const fetchEvents =
    deps?.fetchEvents ?? relayClient.fetchEvents.bind(relayClient);
  const relayPubkey = await getRelayPubkey();
  if (!relayPubkey) {
    throw new Error("Could not verify the relay identity for Project State.");
  }
  const normalizedRelay = relayPubkey.toLowerCase();
  const events = await fetchEvents({
    kinds: [KIND_PROJECT_STATE],
    authors: [normalizedRelay],
    "#a": [coordinate],
    limit: 2,
  });
  if (events.length > 1) {
    throw new Error("The relay returned ambiguous Project State.");
  }
  if (events.length === 0) return null;
  try {
    return parseProjectState(events[0], normalizedRelay, coordinate);
  } catch {
    throw new Error(
      "The relay returned untrusted or unsupported Project State.",
    );
  }
}

function canonicalizePatch(
  patch: ProjectRelatedChannelPatch,
): ProjectRelatedChannelPatch {
  const add = [...patch.add].sort();
  const remove = [...patch.remove].sort();
  if (add.length > MAX_PATCH_CHANNELS || remove.length > MAX_PATCH_CHANNELS) {
    throw new Error("A Project change may add or remove at most 64 channels.");
  }
  if (add.length === 0 && remove.length === 0) {
    throw new Error("A Project related-channel change must not be empty.");
  }
  for (const channelId of [...add, ...remove]) {
    if (!CANONICAL_UUID.test(channelId)) {
      throw new Error("Related channels must use canonical lowercase UUIDs.");
    }
  }
  if (
    new Set(add).size !== add.length ||
    new Set(remove).size !== remove.length
  ) {
    throw new Error("A Project change must not contain duplicate channels.");
  }
  const removeSet = new Set(remove);
  if (add.some((channelId) => removeSet.has(channelId))) {
    throw new Error("A Project change cannot add and remove the same channel.");
  }
  return { add, remove };
}

/** Build the exact NIP-PC kind:47010 command template. */
export function buildProjectRelatedChannelChangeTemplate(
  project: string | Pick<Project, "projectAddress">,
  patch: ProjectRelatedChannelPatch,
  expectedRevision: string,
): ProjectRelatedChannelChangeTemplate {
  const coordinate = projectCoordinate(project);
  parseProjectCoordinate(coordinate);
  if (
    !/^[1-9][0-9]*$/.test(expectedRevision) ||
    BigInt(expectedRevision) > MAX_REVISION
  ) {
    throw new Error("Expected Project revision is not canonical.");
  }
  const canonicalPatch = canonicalizePatch(patch);
  return {
    kind: KIND_PROJECT_CHANGE,
    tags: [
      ["a", coordinate],
      ["expected-revision", expectedRevision],
    ],
    content: JSON.stringify({
      v: 1,
      patch: { related_channels: canonicalPatch },
    }),
  };
}

function isRevisionConflict(error: unknown): boolean {
  return (
    error instanceof Error &&
    error.message.includes("conflict: Project revision is ")
  );
}

/** Publish a related-channel change, retrying one stale-revision conflict. */
export async function changeProjectRelatedChannels(
  project: Pick<Project, "projectAddress">,
  patch: ProjectRelatedChannelPatch,
  deps?: Partial<ChangeProjectRelatedChannelsDeps>,
): Promise<void> {
  const fetchDeps: Partial<ProjectStateDeps> = {
    fetchEvents: deps?.fetchEvents,
    getRelayPubkey: deps?.getRelayPubkey,
  };
  const signEvent = deps?.signEvent ?? signRelayEvent;
  const publishEvent =
    deps?.publishEvent ?? relayClient.publishEvent.bind(relayClient);

  for (let attempt = 0; attempt < 2; attempt += 1) {
    const state = await fetchProjectState(project, fetchDeps);
    if (!state) {
      throw new Error(
        "Project State is not available yet. Refresh and try again.",
      );
    }
    if (state.deleted) {
      throw new Error("This Project has been deleted.");
    }
    const event = await signEvent(
      buildProjectRelatedChannelChangeTemplate(project, patch, state.revision),
    );
    try {
      await publishEvent(
        event,
        "Could not confirm the Project channel change. Refresh and try again.",
        "Failed to change the Project's related channels.",
      );
      return;
    } catch (error) {
      if (attempt === 0 && isRevisionConflict(error)) continue;
      throw error;
    }
  }
}
