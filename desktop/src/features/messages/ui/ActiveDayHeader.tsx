/**
 * Floating active-day label for the virtualized timeline.
 *
 * The in-stream `DayDivider` rows mark each calendar boundary, but once a row
 * scrolls out of the virtual window it unmounts — so a `position: sticky`
 * divider can no longer hold the viewport top, and a sticky element inside the
 * scroll container drifts at scrollTop 0 (it reveals its natural flow offset
 * instead of the pinned offset). This label is therefore portaled by
 * `VirtualizedList` into a non-scrolling overlay container OUTSIDE the scroll
 * element (see `MessageTimeline`), where it pins to a fixed viewport offset and
 * cannot drift as older history prepends above the anchor. It is fed the day of
 * the topmost visible row so the label is always present regardless of which
 * dividers are currently mounted.
 */
export function ActiveDayHeader({ label }: { label: string }) {
  return (
    <p
      aria-hidden
      className="pointer-events-none mx-auto w-fit rounded-full border border-border/70 bg-background/95 px-2.5 py-1 text-2xs font-medium tracking-[0.02em] text-muted-foreground/70 shadow-xs backdrop-blur-sm"
      data-testid="message-timeline-active-day-header"
      data-day-label={label}
    >
      {label}
    </p>
  );
}
