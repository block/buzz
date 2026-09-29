/**
 * Coordinates expand-share dock UI with transcript auto-read.
 * While share fills the stage and the dock chat popover is closed, the
 * covered transcript must not silently clear unread markers.
 */

type Listener = () => void;

let shareExpanded = false;
let dockChatOpen = false;
const listeners = new Set<Listener>();

function emit() {
  for (const listener of listeners) listener();
}

export function setHuddleShareExpanded(next: boolean) {
  if (shareExpanded === next) return;
  shareExpanded = next;
  if (!next) dockChatOpen = false;
  emit();
}

export function setHuddleDockChatOpen(next: boolean) {
  if (dockChatOpen === next) return;
  dockChatOpen = next;
  emit();
}

/** True when the stage covers chat and the dock popover is not reading it. */
export function getHuddleShareBlocksAutoRead() {
  return shareExpanded && !dockChatOpen;
}

export function getHuddleShareExpanded() {
  return shareExpanded;
}

export function getHuddleDockChatOpen() {
  return dockChatOpen;
}

export function subscribeHuddleShareUiState(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
