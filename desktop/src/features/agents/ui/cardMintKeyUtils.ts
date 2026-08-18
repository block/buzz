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
 * Whether the resolved layer is writable from the dialog.
 * Only `"global"` (and unset/`"none"`) can be updated via the dialog seam.
 */
export function isWritableLayer(
  keyLayer: CardMintKeyLayer | undefined,
): boolean {
  return keyLayer === "global" || keyLayer === "none";
}

/**
 * Whether the resolved layer cannot be updated from the dialog (key would be
 * shadowed by the higher-priority layer even if global were updated).
 */
export function isReadOnlyLayer(
  keyLayer: CardMintKeyLayer | undefined,
): boolean {
  return (
    keyLayer === "agent" || keyLayer === "persona" || keyLayer === "process"
  );
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
  return keyLayer === "global" && !editingKey;
}

/** Whether the read-only provenance row should be shown on the mint form. */
export function showReadOnlyRow(
  keyLayer: CardMintKeyLayer | undefined,
  editingKey: boolean,
): boolean {
  return isReadOnlyLayer(keyLayer) && !editingKey;
}

/** The header title for the key-setup panel. */
export function keyPanelTitle(
  keyLayer: CardMintKeyLayer | undefined,
  editingKey: boolean,
): string {
  if (isReadOnlyLayer(keyLayer)) return "OpenAI API key";
  if (keyLayer === "none") return "One-time setup: OpenAI API key";
  return editingKey
    ? "Update OpenAI API key"
    : "One-time setup: OpenAI API key";
}
