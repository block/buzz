import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  buildProjectHomeFromFetcher,
  buildProjectsFromFetcher,
  type FetchProjectEventsExhaustively,
  fetchProjectEventsExhaustively,
} from "./projectEnumeration";
import type { Project } from "./projectModels";
import { markProjectDataAuthoritative } from "./projectSnapshot";

const HIDDEN_PROJECT_CARDS_KEY = "buzz.projects.hidden-cards.v1";

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
  const projects = await buildProjectsFromFetcher(fetcher, {
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
      relayOrigin: getCachedRelayOrigin(),
      hiddenAddresses: new Set(readHiddenProjectCards()),
      viewerPubkey,
    },
  );
  return project ? markProjectDataAuthoritative(project, "relay") : null;
}
