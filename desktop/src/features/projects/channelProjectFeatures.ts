import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

import type { Project, Repository } from "./projectModels";

const STORAGE_KEY_PREFIX = "buzz-channel-project-features.v1";
const MAX_CHANNEL_PREFERENCES = 1_000;
export const CHANNEL_PROJECT_FEATURES_CHANGED_EVENT =
  "buzz:channel-project-features-changed";

export type ChannelProjectFeature =
  | "tasks"
  | "breakouts"
  | "reviews"
  | "repositories";

export type ChannelProjectFeaturePreferences = {
  tasks?: boolean;
  breakouts?: boolean;
  reviews?: boolean;
  repositories?: boolean;
  breakoutSectionId?: string;
};

type ChannelProjectFeatureStore = {
  version: 1;
  channels: Record<string, ChannelProjectFeaturePreferences>;
};

const EMPTY_PREFERENCES: ChannelProjectFeaturePreferences = Object.freeze({});

export function channelProjectFeatureStorageKey(
  pubkey: string,
  relayUrl: string,
) {
  return `${STORAGE_KEY_PREFIX}:${pubkey.toLowerCase()}:${encodeURIComponent(
    normalizeRelayUrl(relayUrl),
  )}`;
}

function parsePreferences(value: unknown): ChannelProjectFeaturePreferences {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const candidate = value as Record<string, unknown>;
  return {
    ...(typeof candidate.tasks === "boolean" ? { tasks: candidate.tasks } : {}),
    ...(typeof candidate.breakouts === "boolean"
      ? { breakouts: candidate.breakouts }
      : {}),
    ...(typeof candidate.reviews === "boolean"
      ? { reviews: candidate.reviews }
      : {}),
    ...(typeof candidate.repositories === "boolean"
      ? { repositories: candidate.repositories }
      : {}),
    ...(typeof candidate.breakoutSectionId === "string" &&
    candidate.breakoutSectionId.length > 0
      ? { breakoutSectionId: candidate.breakoutSectionId }
      : {}),
  };
}

export function parseChannelProjectFeatureStore(
  value: unknown,
): ChannelProjectFeatureStore {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { version: 1, channels: {} };
  }
  const candidate = value as Record<string, unknown>;
  if (
    candidate.version !== 1 ||
    !candidate.channels ||
    typeof candidate.channels !== "object" ||
    Array.isArray(candidate.channels)
  ) {
    return { version: 1, channels: {} };
  }
  return {
    version: 1,
    channels: Object.fromEntries(
      Object.entries(candidate.channels as Record<string, unknown>)
        .filter(([channelId]) => channelId.length > 0)
        .slice(-MAX_CHANNEL_PREFERENCES)
        .map(([channelId, preferences]) => [
          channelId,
          parsePreferences(preferences),
        ]),
    ),
  };
}

function readStore(pubkey: string, relayUrl: string) {
  try {
    const raw = window.localStorage.getItem(
      channelProjectFeatureStorageKey(pubkey, relayUrl),
    );
    return parseChannelProjectFeatureStore(raw ? JSON.parse(raw) : null);
  } catch {
    return { version: 1, channels: {} } satisfies ChannelProjectFeatureStore;
  }
}

export function readChannelProjectFeaturePreferences(
  pubkey: string | undefined,
  relayUrl: string | undefined,
  channelId: string,
) {
  if (!pubkey || !relayUrl) return EMPTY_PREFERENCES;
  return readStore(pubkey, relayUrl).channels[channelId] ?? EMPTY_PREFERENCES;
}

export function writeChannelProjectFeaturePreferences(
  pubkey: string,
  relayUrl: string,
  channelId: string,
  patch: Partial<ChannelProjectFeaturePreferences>,
) {
  const key = channelProjectFeatureStorageKey(pubkey, relayUrl);
  try {
    const store = readStore(pubkey, relayUrl);
    const next = parsePreferences({
      ...store.channels[channelId],
      ...patch,
    });
    // TODO: Replace this browser-local POC state with shared persisted
    // capability metadata if the channel-first model is validated.
    window.localStorage.setItem(
      key,
      JSON.stringify(
        parseChannelProjectFeatureStore({
          version: 1,
          channels: { ...store.channels, [channelId]: next },
        }),
      ),
    );
    window.dispatchEvent(
      new window.CustomEvent(CHANNEL_PROJECT_FEATURES_CHANGED_EVENT, {
        detail: { key },
      }),
    );
    return next;
  } catch {
    return null;
  }
}

export function findChannelProject(
  projects: readonly Project[],
  channelId: string,
) {
  return (
    projects.find((project) => project.projectChannelId === channelId) ??
    projects.find(
      (project) =>
        project.legacy &&
        project.repositories.some(
          (repository) => repository.channelId === channelId,
        ),
    ) ??
    null
  );
}

export function projectPrimaryRepository(project: Project | null) {
  if (!project) return null;
  return (
    project.repositories.find(
      (repository) =>
        repository.repoAddress === project.primaryRepositoryAddress,
    ) ??
    project.repositories[0] ??
    null
  );
}

export function projectRelatedRepositories(project: Project | null) {
  if (!project) return [];
  const primary = projectPrimaryRepository(project);
  return project.repositories.filter(
    (repository) => repository.repoAddress !== primary?.repoAddress,
  );
}

export function projectRelatedChannelIds(
  project: Project | null,
  rootChannelId: string,
) {
  if (!project) return [];
  return [
    ...new Set(
      [
        ...(project.relatedChannelIds ?? []),
        ...project.repositories.map((repository: Repository) =>
          repository.channelId?.trim(),
        ),
      ].filter(
        (channelId): channelId is string =>
          Boolean(channelId) && channelId !== rootChannelId,
      ),
    ),
  ];
}

export function channelProjectFeatureEnabled({
  feature,
  hasExistingData,
  preferences,
}: {
  feature: ChannelProjectFeature;
  hasExistingData: boolean;
  preferences: ChannelProjectFeaturePreferences;
}) {
  return hasExistingData || preferences[feature] === true;
}
