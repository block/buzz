export function SearchResultTrailing({
  channelId,
  isSelected,
  trailingLabel,
  unreadCount,
}: {
  channelId?: string;
  isSelected: boolean;
  trailingLabel: string | null;
  unreadCount: number;
}) {
  return (
    <>
      {channelId && unreadCount > 0 ? (
        <span
          className="flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full bg-primary px-1.5 text-2xs font-semibold text-primary-foreground"
          data-testid={`search-unread-count-${channelId}`}
          title={`${unreadCount} unread message${unreadCount === 1 ? "" : "s"}`}
        >
          {Math.min(unreadCount, 99)}
        </span>
      ) : null}
      {isSelected ? (
        <kbd className="shrink-0 rounded border border-border/70 bg-background/70 px-1.5 py-0.5 text-2xs text-muted-foreground">
          Enter
        </kbd>
      ) : null}
      {trailingLabel ? (
        <span className="shrink-0 text-2xs text-muted-foreground/75">
          {trailingLabel}
        </span>
      ) : null}
    </>
  );
}
