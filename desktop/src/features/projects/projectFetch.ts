import { getRelaySelf } from "@/features/moderation/lib/relaySelf";
import { relayClient } from "@/shared/api/relayClient";
import { KIND_PROJECT_RELATED_CHANNEL_SNAPSHOT } from "@/shared/constants/kinds";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  buildProjectHomeFromFetcher,
  buildProjectsFromFetcher,
  type FetchProjectEventsExhaustively,
  type FetchProjectRelatedChannelSnapshots,
  fetchProjectEventsExhaustively,
} from "./projectEnumeration";
import type { Project } from "./projectModels";
import { projectRelatedChannelSnapshotD } from "./projectRelatedChannelSnapshot";
import { markProjectDataAuthoritative } from "./projectSnapshot";

const HIDDEN_PROJECT_CARDS_KEY = "buzz.projects.hidden-cards.v1";

async function trustedSnapshotFetcher(
  signal?: AbortSignal,
): Promise<FetchProjectRelatedChannelSnapshots | undefined> {
  signal?.throwIfAborted();
  const relaySelf = await getRelaySelf();
  if (!relaySelf) return undefined;
  return async (projectAddresses) => {
    signal?.throwIfAborted();
    const snapshotDTags = await Promise.all(
      projectAddresses.map(projectRelatedChannelSnapshotD),
    );
    signal?.throwIfAborted();
    return relayClient.fetchEvents({
      kinds: [KIND_PROJECT_RELATED_CHANNEL_SNAPSHOT],
      authors: [relaySelf],
      "#d": snapshotDTags,
      limit: projectAddresses.length,
    });
  };
}

function readHiddenProjectCards(): string[] {
  if (typeof window === "undefined") {
    return [];
  }

  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(HIDDEN_PROJECT_CARDS_KEY) ?? "[]",
    );
    return Array.isArray(parsed)
      ? parsed.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

/** Enumerates the projects visible to the current relay identity. */
export async function fetchProjects(
  fetchExhaustively?: FetchProjectEventsExhaustively,
  signal?: AbortSignal,
): Promise<Project[]> {
  // Delegates to `buildProjectsFromFetcher` in `projectEnumeration.ts`, which
  // is the pure, Tauri-free core of this operation. Its javadoc explains
  // fail-closed tombstones and NIP-OA owner-deletion suppression.
  const viewerPubkey = await getIdentity()
    .then((identity) => identity.pubkey)
    .catch(() => undefined);
  const fetcher: FetchProjectEventsExhaustively =
    fetchExhaustively ??
    ((kinds, extraFilter) =>
      fetchProjectEventsExhaustively(kinds, extraFilter, undefined, signal));
  const fetchRelatedChannelSnapshots = fetchExhaustively
    ? undefined
    : await trustedSnapshotFetcher(signal);
  const projects = await buildProjectsFromFetcher(fetcher, {
    fetchRelatedChannelSnapshots,
    relayOrigin: getCachedRelayOrigin(),
    hiddenAddresses: new Set(readHiddenProjectCards()),
    viewerPubkey,
  });
  return projects.map((project) =>
    markProjectDataAuthoritative(project, "relay"),
  );
}

/** Resolves the active channel's project home with a scoped relay query. */
export async function fetchProjectHomeForChannel(
  channelId: string,
  signal?: AbortSignal,
): Promise<Project | null> {
  const viewerPubkey = await getIdentity()
    .then((identity) => identity.pubkey)
    .catch(() => undefined);
  const project = await buildProjectHomeFromFetcher(
    (kinds, extraFilter) =>
      fetchProjectEventsExhaustively(kinds, extraFilter, undefined, signal),
    channelId,
    {
      fetchRelatedChannelSnapshots: await trustedSnapshotFetcher(signal),
      relayOrigin: getCachedRelayOrigin(),
      hiddenAddresses: new Set(readHiddenProjectCards()),
      viewerPubkey,
    },
  );
  return project ? markProjectDataAuthoritative(project, "relay") : null;
}
