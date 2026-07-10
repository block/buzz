/**
 * Global event for requesting that the Edit Agent dialog open for a specific
 * agent pubkey, with an optional field-focus target.
 *
 * Pattern mirrors `openCreateAgentEvent.ts`. The card (or any caller outside
 * a UserProfilePanel instance) dispatches the event; UserProfilePanel
 * subscribes and opens the dialog when its current pubkey matches, forwarding
 * the focus target so the dialog can scroll/focus the relevant field.
 *
 * Callers typically also call `openProfilePanel(pubkey)` from ProfilePanel-
 * Context to ensure the panel is visible before the event fires.
 */

const OPEN_EDIT_AGENT_EVENT = "buzz:open-edit-agent";

/**
 * Optional focus target for the Edit Agent dialog.
 *
 * - `env_key`: scroll the env-vars editor to the matching required-key row
 *   and focus its value input.
 * - `normalized_field`: focus the provider (`agent-provider`) or model
 *   (`agent-model`) dropdown that corresponds to the missing field.
 */
export type EditAgentFocusTarget =
  | { type: "env_key"; key: string }
  | { type: "normalized_field"; field: string };

type OpenEditAgentDetail = { pubkey: string; focus?: EditAgentFocusTarget };

let pendingEditAgentPubkey: string | null = null;
let pendingEditAgentFocus: EditAgentFocusTarget | undefined;

export function requestOpenEditAgent(
  pubkey: string,
  focus?: EditAgentFocusTarget,
) {
  pendingEditAgentPubkey = pubkey;
  pendingEditAgentFocus = focus;
  window.dispatchEvent(
    new CustomEvent<OpenEditAgentDetail>(OPEN_EDIT_AGENT_EVENT, {
      detail: { pubkey, focus },
    }),
  );
}

/**
 * Consume the pending open-edit-agent request for `pubkey`.
 *
 * Returns the focus target when a matching pending request exists (clearing
 * it), `true` when a matching request exists with no focus target, or
 * `false` when no matching request is pending.
 */
export function consumePendingOpenEditAgent(
  pubkey: string,
): EditAgentFocusTarget | true | false {
  if (
    pendingEditAgentPubkey !== null &&
    pendingEditAgentPubkey.toLowerCase() === pubkey.toLowerCase()
  ) {
    pendingEditAgentPubkey = null;
    const focus = pendingEditAgentFocus;
    pendingEditAgentFocus = undefined;
    return focus ?? true;
  }
  return false;
}

export function subscribeOpenEditAgent(
  pubkey: string,
  handler: (focus?: EditAgentFocusTarget) => void,
): () => void {
  function handleEvent(event: Event) {
    const detail = (event as CustomEvent<OpenEditAgentDetail>).detail;
    if (detail.pubkey.toLowerCase() === pubkey.toLowerCase()) {
      pendingEditAgentPubkey = null;
      pendingEditAgentFocus = undefined;
      handler(detail.focus);
    }
  }

  window.addEventListener(OPEN_EDIT_AGENT_EVENT, handleEvent);
  return () => {
    window.removeEventListener(OPEN_EDIT_AGENT_EVENT, handleEvent);
  };
}
