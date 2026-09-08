import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { signRelayEvent } from "@/shared/api/tauri";
import {
  KIND_DELETION,
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Project } from "./projectModels";

export type DeleteProjectEventTemplate = {
  kind: number;
  content: string;
  createdAt: number;
  tags: string[][];
};

/**
 * Buzz lets a human owner manage content authored by their NIP-OA agent, just
 * as it does for agent-owned channels. The profile ownership evidence mirrors
 * the relay authority used for the owner-signed tombstone.
 */
export function canDeleteProject(
  project: Pick<Project, "owner">,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): boolean {
  if (!currentPubkey) return false;

  const owner = normalizePubkey(project.owner);
  return (
    owner === normalizePubkey(currentPubkey) ||
    ownsAuthorAgent(profiles?.[owner], currentPubkey)
  );
}

/** Build a tombstone that dominates the exact live coordinate head. */
export function buildProjectDeletionTemplate(
  project: Pick<Project, "name" | "projectAddress">,
  liveHead: Pick<RelayEvent, "created_at">,
  nowSeconds = Math.floor(Date.now() / 1_000),
): DeleteProjectEventTemplate {
  return {
    kind: KIND_DELETION,
    content: `Delete project ${project.name}`,
    createdAt: Math.max(nowSeconds, liveHead.created_at + 1),
    tags: [["a", project.projectAddress]],
  };
}

type ProjectDeletionFetchEvents = (filter: {
  kinds: number[];
  authors: string[];
  "#d": string[];
  limit: number;
}) => Promise<RelayEvent[]>;

type ProjectDeletionDeps = {
  fetchEvents: ProjectDeletionFetchEvents;
  nowSeconds: () => number;
  publishEvent: (
    event: RelayEvent,
    timeoutMessage: string,
    failureMessage: string,
  ) => Promise<void>;
  signEvent: (input: DeleteProjectEventTemplate) => Promise<RelayEvent>;
};

/** Delete the exact live project coordinate and detect a concurrent replacement. */
export async function deleteProject(
  project: Project,
  deps?: Partial<ProjectDeletionDeps>,
): Promise<void> {
  const {
    fetchEvents = relayClient.fetchEvents.bind(relayClient),
    nowSeconds = () => Math.floor(Date.now() / 1_000),
    publishEvent = relayClient.publishEvent.bind(relayClient),
    signEvent = signRelayEvent,
  } = deps ?? {};
  const filter = {
    kinds: [
      project.legacy ? KIND_REPO_ANNOUNCEMENT : KIND_PROJECT_ANNOUNCEMENT,
    ],
    authors: [project.owner.toLowerCase()],
    "#d": [project.dtag],
    limit: 1,
  };
  const [liveHead] = await fetchEvents(filter);
  if (!liveHead) {
    throw new Error(
      "Could not find this project on the relay. Refresh and try again.",
    );
  }

  const event = await signEvent(
    buildProjectDeletionTemplate(project, liveHead, nowSeconds()),
  );
  await publishEvent(
    event,
    "Could not confirm whether the project was deleted. Projects were refreshed.",
    "Failed to delete project.",
  );

  if ((await fetchEvents(filter)).length > 0) {
    throw new Error(
      "This project was updated while it was being deleted. Refresh and try again.",
    );
  }
}
