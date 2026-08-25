export const BUSINESS_DOCK_PREFERENCES_KEY =
  "buzz.business-dock.preferences.v2";

export type BusinessDockPreferences = {
  followConversation: boolean;
  pinned: boolean;
};

export const DEFAULT_BUSINESS_DOCK_PREFERENCES: BusinessDockPreferences = {
  followConversation: true,
  pinned: false,
};

type StorageLike = Pick<Storage, "getItem" | "setItem">;

export function readBusinessDockPreferences(
  storage: StorageLike | null | undefined,
): BusinessDockPreferences {
  if (!storage) return DEFAULT_BUSINESS_DOCK_PREFERENCES;
  try {
    const value = JSON.parse(
      storage.getItem(BUSINESS_DOCK_PREFERENCES_KEY) ?? "null",
    );
    return {
      followConversation:
        typeof value?.followConversation === "boolean"
          ? value.followConversation
          : true,
      pinned: typeof value?.pinned === "boolean" ? value.pinned : false,
    };
  } catch {
    return DEFAULT_BUSINESS_DOCK_PREFERENCES;
  }
}

export function saveBusinessDockPreferences(
  storage: StorageLike | null | undefined,
  preferences: BusinessDockPreferences,
): void {
  if (!storage) return;
  try {
    storage.setItem(BUSINESS_DOCK_PREFERENCES_KEY, JSON.stringify(preferences));
  } catch {
    // Storage failures must not disable Business Dock navigation.
  }
}
