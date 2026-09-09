import { JUMP_TO_UNREAD_EVENT } from "@/shared/lib/keyboard-shortcuts";

// Latch Threads requests until navigation mounts HomeView.

const OPEN_INBOX_THREADS_EVENT = "buzz:open-inbox-threads";

let pendingOpenInboxThreads = false;

export function requestOpenInboxThreads() {
  pendingOpenInboxThreads = true;
  window.dispatchEvent(new CustomEvent(OPEN_INBOX_THREADS_EVENT));
}

export function consumePendingOpenInboxThreads(): boolean {
  const pending = pendingOpenInboxThreads;
  pendingOpenInboxThreads = false;
  return pending;
}

export function subscribeOpenInboxThreads(handler: () => void) {
  function handleOpenInboxThreads() {
    pendingOpenInboxThreads = false;
    handler();
  }

  window.addEventListener(OPEN_INBOX_THREADS_EVENT, handleOpenInboxThreads);
  return () => {
    window.removeEventListener(
      OPEN_INBOX_THREADS_EVENT,
      handleOpenInboxThreads,
    );
  };
}

export function requestJumpToUnread() {
  window.dispatchEvent(new CustomEvent(JUMP_TO_UNREAD_EVENT));
}
