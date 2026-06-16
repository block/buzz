import type { DesktopNotificationTarget } from "@/features/notifications/lib/desktop";
import type { SearchHit } from "@/shared/api/types";

export type AppView =
  | "home"
  | "channel"
  | "agents"
  | "workflows"
  | "pulse"
  | "projects";

const WINDOW_DRAG_HANDLE_HEIGHT = 44;
const WINDOW_DRAG_INTERACTIVE_SELECTOR =
  'button, a, input, textarea, select, [role="button"], [contenteditable="true"]';

export function isWindowDragHandleEvent(event: MouseEvent | PointerEvent) {
  if (event.clientY > WINDOW_DRAG_HANDLE_HEIGHT) {
    return false;
  }

  const target = event.target;
  return !(
    target instanceof Element &&
    target.closest(WINDOW_DRAG_INTERACTIVE_SELECTOR)
  );
}

export function toSearchHit(
  target: DesktopNotificationTarget,
): SearchHit | null {
  if (!target.eventId) {
    return null;
  }

  return {
    eventId: target.eventId,
    content: target.content ?? "",
    kind: target.kind ?? 9,
    pubkey: target.pubkey ?? "",
    channelId: target.channelId,
    channelName: target.channelName ?? null,
    createdAt: target.createdAt ?? Math.floor(Date.now() / 1_000),
    score: 0,
    threadRootId: target.threadRootId ?? null,
  };
}

export function deriveShellRoute(pathname: string): {
  selectedChannelId: string | null;
  selectedView: AppView;
} {
  if (pathname.startsWith("/channels/")) {
    const [, , rawChannelId] = pathname.split("/");
    return {
      selectedChannelId: rawChannelId ? decodeURIComponent(rawChannelId) : null,
      selectedView: "channel",
    };
  }

  if (pathname === "/agents") {
    return {
      selectedChannelId: null,
      selectedView: "agents",
    };
  }

  if (pathname === "/workflows" || pathname.startsWith("/workflows/")) {
    return {
      selectedChannelId: null,
      selectedView: "workflows",
    };
  }

  if (pathname === "/projects" || pathname.startsWith("/projects/")) {
    return {
      selectedChannelId: null,
      selectedView: "projects",
    };
  }

  if (pathname === "/pulse") {
    return {
      selectedChannelId: null,
      selectedView: "pulse",
    };
  }

  return {
    selectedChannelId: null,
    selectedView: "home",
  };
}
