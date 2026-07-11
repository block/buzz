import { isThreadReply } from "@/features/messages/lib/threading";
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
const TAURI_DRAG_REGION_ATTR = "data-tauri-drag-region";
const WINDOW_DRAG_INTERACTIVE_SELECTOR =
  'button, a, input, textarea, select, label, summary, [role="button"], [role="link"], [role="menuitem"], [role="tab"], [role="checkbox"], [role="radio"], [role="switch"], [role="option"], [contenteditable="true"], [tabindex]:not([tabindex="-1"])';

const CLICKABLE_TAGS = new Set([
  "A",
  "BUTTON",
  "INPUT",
  "SELECT",
  "TEXTAREA",
  "LABEL",
  "SUMMARY",
]);
const INTERACTIVE_ROLES = new Set([
  "button",
  "link",
  "menuitem",
  "tab",
  "checkbox",
  "radio",
  "switch",
  "option",
]);

function isClickableElement(element: HTMLElement) {
  return (
    CLICKABLE_TAGS.has(element.tagName) ||
    (element.hasAttribute("contenteditable") &&
      element.getAttribute("contenteditable") !== "false") ||
    (element.hasAttribute("tabindex") &&
      element.getAttribute("tabindex") !== "-1") ||
    INTERACTIVE_ROLES.has(element.getAttribute("role") ?? "")
  );
}

function isTauriDragRegionEvent(event: MouseEvent | PointerEvent) {
  const path = event.composedPath();
  const directTarget = path[0];

  for (const item of path) {
    if (!(item instanceof HTMLElement)) continue;

    const attr = item.getAttribute(TAURI_DRAG_REGION_ATTR);

    if (isClickableElement(item) && attr === null) return false;
    if (attr === null) continue;
    if (attr === "false") return false;
    if (attr === "deep") return true;
    if (attr === "" || attr === "true") return item === directTarget;
  }

  return false;
}

export function isWindowDragHandleEvent(event: MouseEvent | PointerEvent) {
  if (isTauriDragRegionEvent(event)) {
    return true;
  }

  if (event.clientY > WINDOW_DRAG_HANDLE_HEIGHT) {
    return false;
  }

  const target = event.target;
  return !(
    target instanceof Element &&
    target.closest(WINDOW_DRAG_INTERACTIVE_SELECTOR)
  );
}

export function shouldBounceForChannelNotification(tags: string[][]): boolean {
  return !isThreadReply(tags);
}

/**
 * Schedule the relay preconnect on the next tick.
 *
 * The relay handshake (WS connect + NIP-42 auth) is safe to start the moment
 * `AppShell` mounts: the router only renders `AppShell` once onboarding has
 * settled to `stage === "ready"`, so the signing identity is already present.
 * We still defer by a single macrotask (`delay = 0`) rather than running
 * synchronously so the handshake does not contend with the first paint.
 *
 * Returns a cancel function that prevents `run` from firing if it has not
 * already run — used by the effect cleanup so an unmount before the tick does
 * not open a socket.
 */
export function schedulePreconnect(
  run: () => void,
  scheduler: {
    setTimeout: (callback: () => void, delayMs: number) => number;
    clearTimeout: (handle: number) => void;
  } = globalThis,
): () => void {
  let cancelled = false;
  const handle = scheduler.setTimeout(() => {
    if (!cancelled) {
      run();
    }
  }, 0);

  return () => {
    cancelled = true;
    scheduler.clearTimeout(handle);
  };
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
