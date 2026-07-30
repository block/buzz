/**
 * Pure decision logic for the "clear an edit to empty = delete the message"
 * composer shortcut. Kept free of React so the rule can be unit-tested
 * directly.
 */

/**
 * Resolve which message a submitted empty edit should delete.
 *
 * Returns the event id to delete, or `null` when the deletion must not
 * proceed — either no message is loaded for editing (blank/absent id) or no
 * delete handler is wired. In the no-handler case an empty edit stays a no-op
 * rather than destroying anything, preserving the historical guard.
 *
 * Called from the composer's submit path when an edit is cleared to empty: a
 * non-null result is handed straight to the existing delete handler (the same
 * one the "Delete message" button uses), with no separate confirmation UI.
 */
export function resolveEmptyEditDelete(
  editTargetId: string | null | undefined,
  hasDeleteHandler: boolean,
): string | null {
  if (!hasDeleteHandler || !editTargetId) {
    return null;
  }
  return editTargetId;
}
