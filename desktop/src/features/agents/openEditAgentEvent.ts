/**
 * Global event for requesting that the Edit Agent dialog open for a specific
 * agent pubkey.
 *
 * Pattern mirrors `openCreateAgentEvent.ts`. The card (or any caller outside
 * a UserProfilePanel instance) dispatches the event; UserProfilePanel
 * subscribes and opens the dialog when its current pubkey matches.
 *
 * Callers typically also call `openProfilePanel(pubkey)` from ProfilePanel-
 * Context to ensure the panel is visible before the event fires.
 */

const OPEN_EDIT_AGENT_EVENT = "buzz:open-edit-agent";

let pendingEditAgentPubkey: string | null = null;

export function requestOpenEditAgent(pubkey: string) {
  pendingEditAgentPubkey = pubkey;
  window.dispatchEvent(
    new CustomEvent<string>(OPEN_EDIT_AGENT_EVENT, { detail: pubkey }),
  );
}

export function consumePendingOpenEditAgent(pubkey: string): boolean {
  if (
    pendingEditAgentPubkey !== null &&
    pendingEditAgentPubkey.toLowerCase() === pubkey.toLowerCase()
  ) {
    pendingEditAgentPubkey = null;
    return true;
  }
  return false;
}

export function subscribeOpenEditAgent(
  pubkey: string,
  handler: () => void,
): () => void {
  function handleEvent(event: Event) {
    const detail = (event as CustomEvent<string>).detail;
    if (detail.toLowerCase() === pubkey.toLowerCase()) {
      pendingEditAgentPubkey = null;
      handler();
    }
  }

  window.addEventListener(OPEN_EDIT_AGENT_EVENT, handleEvent);
  return () => {
    window.removeEventListener(OPEN_EDIT_AGENT_EVENT, handleEvent);
  };
}
