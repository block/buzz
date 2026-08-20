import type { ThreadRailPin } from "./threadRailStorage";

export function threadRailRootIdFromSearch(search: unknown): string | null {
  if (!search || typeof search !== "object") return null;
  const route = search as { thread?: unknown; threadRootId?: unknown };
  const rootId = route.thread ?? route.threadRootId;
  return typeof rootId === "string" ? rootId : null;
}

export function threadRailPinToChannelNavigation(pin: ThreadRailPin) {
  return {
    channelId: pin.channelId,
    thread: pin.rootId,
    messageId: pin.returnAnchorId ?? pin.rootId,
    threadRail: true,
    threadRootId: pin.rootId,
  };
}

export function isThreadRailPinActive(
  pin: ThreadRailPin,
  selectedChannelId: string | null,
  openThreadRootId: string | null,
): boolean {
  return pin.channelId === selectedChannelId && pin.rootId === openThreadRootId;
}
