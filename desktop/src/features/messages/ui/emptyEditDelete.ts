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
 * Used at two moments with the same rule: deciding whether to surface the
 * confirmation on submit, and resolving the target when the user confirms.
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
