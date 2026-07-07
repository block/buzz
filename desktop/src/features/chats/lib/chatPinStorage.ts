import * as React from "react";

// Workspace-scoped pinned chat ids. Local convenience state (mirrors
// chatProjectStorage): pins order the sidebar only, so they never need to
// sync across devices.
const STORAGE_PREFIX = "buzz:chat-pins:v1";
const STORAGE_EVENT = "buzz:chat-pins-changed";

function storageKey(workspaceId: string | null | undefined) {
  return `${STORAGE_PREFIX}:${workspaceId ?? "default"}`;
}

function readStoredChatPins(workspaceId: string | null | undefined): string[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(storageKey(workspaceId));
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

function writeStoredChatPins(
  workspaceId: string | null | undefined,
  chatIds: string[],
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(
      storageKey(workspaceId),
      JSON.stringify(chatIds),
    );
    window.dispatchEvent(new CustomEvent(STORAGE_EVENT));
  } catch {
    // Pins are a convenience layer; ignore unavailable storage.
  }
}

export function toggleStoredChatPin(
  workspaceId: string | null | undefined,
  chatId: string,
) {
  const pins = readStoredChatPins(workspaceId);
  writeStoredChatPins(
    workspaceId,
    pins.includes(chatId)
      ? pins.filter((id) => id !== chatId)
      : [chatId, ...pins],
  );
}

export function useStoredChatPins(
  workspaceId: string | null | undefined,
): ReadonlySet<string> {
  const [pins, setPins] = React.useState<string[]>(() =>
    readStoredChatPins(workspaceId),
  );

  React.useEffect(() => {
    const refresh = () => setPins(readStoredChatPins(workspaceId));
    refresh();
    window.addEventListener(STORAGE_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(STORAGE_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, [workspaceId]);

  return React.useMemo(() => new Set(pins), [pins]);
}
