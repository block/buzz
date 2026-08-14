import type { CardMintKeyLayer } from "@/shared/api/tauriPersonas";

/**
 * Pure derivations for the key-setup panel visibility in `AgentCardMintDialog`.
 *
 * Extracted so that: (a) the component imports and uses the exact same logic
 * as the tests verify, and (b) changes to panel conditions are caught by
 * test failures rather than silently diverging.
 */

/** Whether the key panel should be shown (setup or user-initiated update only). */
export function showKeyPanel(
  keyLayer: CardMintKeyLayer | undefined,
  editingKey: boolean,
): boolean {
  return keyLayer === "none" || editingKey;
}

/**
 * Whether the dialog can save a dedicated custom-card key. Any resolved
 * fallback can be safely overridden because the save never mutates that
 * source layer.
 */
export function isWritableLayer(
  keyLayer: CardMintKeyLayer | undefined,
): boolean {
  return keyLayer !== undefined;
}

/**
 * No existing fallback is read-only: a dedicated card key takes precedence
 * without changing the fallback source.
 */
export function isReadOnlyLayer(
  _keyLayer: CardMintKeyLayer | undefined,
): boolean {
  return false;
}

/** Whether the "Cancel" button should be shown (update mode only, with a mint form to return to). */
export function showCancelButton(
  keyLayer: CardMintKeyLayer | undefined,
  editingKey: boolean,
): boolean {
  return editingKey && keyLayer !== "none";
}

/** Whether the "Using your saved OpenAI key · Update" status row should be shown. */
export function showKeyStatusRow(
  keyLayer: CardMintKeyLayer | undefined,
  editingKey: boolean,
): boolean {
  return keyLayer !== undefined && keyLayer !== "none" && !editingKey;
}

/** Whether the read-only provenance row should be shown on the mint form. */
export function showReadOnlyRow(
  _keyLayer: CardMintKeyLayer | undefined,
  _editingKey: boolean,
): boolean {
  return false;
}

/** The header title for the key-setup panel. */
export function keyPanelTitle(
  keyLayer: CardMintKeyLayer | undefined,
  _editingKey: boolean,
): string {
  if (keyLayer === "none") return "One-time setup: OpenAI API key";
  return "Update OpenAI API key";
}
