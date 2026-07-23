import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

const STORAGE_KEY_PREFIX = "buzz-activity-view.v1";

export type ActivityViewId =
  | "all"
  | "mention"
  | "thread"
  | "needs_action"
  | "agent_activity"
  | "reminders"
  | "drafts"
  | "custom";

export type ActivityCustomView = {
  dms: boolean;
  mentions: boolean;
  threads: boolean;
  needsAction: boolean;
  agentReplies: boolean;
  dueReminders: boolean;
  drafts: boolean;
};

export type ActivityViewPreferences = {
  version: 1;
  defaultView: ActivityViewId;
  custom: ActivityCustomView;
};

export const DEFAULT_ACTIVITY_CUSTOM_VIEW: ActivityCustomView = Object.freeze({
  dms: true,
  mentions: true,
  threads: true,
  needsAction: true,
  agentReplies: true,
  dueReminders: true,
  drafts: true,
});

export const DEFAULT_ACTIVITY_VIEW_PREFERENCES: ActivityViewPreferences =
  Object.freeze({
    version: 1,
    defaultView: "all",
    custom: DEFAULT_ACTIVITY_CUSTOM_VIEW,
  });

const ACTIVITY_VIEW_IDS = new Set<ActivityViewId>([
  "all",
  "mention",
  "thread",
  "needs_action",
  "agent_activity",
  "reminders",
  "drafts",
  "custom",
]);

function isActivityViewId(value: unknown): value is ActivityViewId {
  return (
    typeof value === "string" && ACTIVITY_VIEW_IDS.has(value as ActivityViewId)
  );
}

function parseCustomView(value: unknown): ActivityCustomView {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { ...DEFAULT_ACTIVITY_CUSTOM_VIEW };
  }

  const candidate = value as Record<string, unknown>;
  return Object.fromEntries(
    Object.entries(DEFAULT_ACTIVITY_CUSTOM_VIEW).map(([key, fallback]) => [
      key,
      typeof candidate[key] === "boolean" ? candidate[key] : fallback,
    ]),
  ) as ActivityCustomView;
}

export function parseActivityViewPreferences(
  value: unknown,
): ActivityViewPreferences | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }

  const candidate = value as Record<string, unknown>;
  if (candidate.version !== 1) return null;

  return {
    version: 1,
    defaultView: isActivityViewId(candidate.defaultView)
      ? candidate.defaultView
      : "all",
    custom: parseCustomView(candidate.custom),
  };
}

export function activityViewStorageKey(
  pubkey: string,
  relayUrl?: string,
): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function readActivityViewPreferences(
  pubkey: string,
  relayUrl?: string,
): ActivityViewPreferences {
  try {
    const raw = window.localStorage.getItem(
      activityViewStorageKey(pubkey, relayUrl),
    );
    if (!raw) return DEFAULT_ACTIVITY_VIEW_PREFERENCES;
    return (
      parseActivityViewPreferences(JSON.parse(raw)) ??
      DEFAULT_ACTIVITY_VIEW_PREFERENCES
    );
  } catch {
    return DEFAULT_ACTIVITY_VIEW_PREFERENCES;
  }
}

export function writeActivityViewPreferences(
  pubkey: string,
  preferences: ActivityViewPreferences,
  relayUrl?: string,
): boolean {
  try {
    window.localStorage.setItem(
      activityViewStorageKey(pubkey, relayUrl),
      JSON.stringify(preferences),
    );
    return true;
  } catch {
    return false;
  }
}
