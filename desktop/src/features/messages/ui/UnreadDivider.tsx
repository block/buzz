/**
 * Inline "New" divider rendered directly above the oldest unread top-level
 * message, mirroring Slack's read/unread boundary. Computed from the
 * channel's read frontier as it stood when the channel was opened.
 */
export function UnreadDivider() {
  return (
    <section
      aria-label="New messages"
      className="relative flex items-center py-1"
      data-testid="message-unread-divider"
    >
      <div className="h-px flex-1 bg-primary/40" />
      <span className="shrink-0 px-2 text-2xs font-semibold uppercase tracking-[0.04em] text-primary">
        New
      </span>
      <div className="h-px flex-1 bg-primary/40" />
    </section>
  );
}
