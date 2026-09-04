/**
 * Whether the canvas ingress row should be shown.
 *
 * Existence is keyed on `eventId` (a persisted kind:40100 revision exists on
 * the relay), not on content length — a restore to empty still leaves a live
 * revision, and a read-only member must be able to reach it. `canEditNarrative`
 * independently grants access so editors can seed the first revision.
 */
export function canvasIngressOpen(
  eventId: string | null | undefined,
  canEditNarrative: boolean,
): boolean {
  return eventId != null || canEditNarrative;
}
