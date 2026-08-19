import type { ThreadRailPin } from "./threadRailStorage";

type ThreadRailAnchorTarget = { id: string; rootId?: string | null };

/** Returns a local nested reply anchor only when it belongs to a pinned open thread. */
export function getThreadRailAnchorUpdate(
  pins: ThreadRailPin[],
  channelId: string | null,
  rootId: string | null,
  target: ThreadRailAnchorTarget,
): { pin: ThreadRailPin; returnAnchorId: string } | null {
  if (
    !channelId ||
    !rootId ||
    target.id === rootId ||
    target.rootId !== rootId
  ) {
    return null;
  }
  const pin = pins.find(
    (candidate) =>
      candidate.channelId === channelId && candidate.rootId === rootId,
  );
  return pin ? { pin, returnAnchorId: target.id } : null;
}
