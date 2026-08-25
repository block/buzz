import { topChromeInset } from "@/shared/layout/chromeLayout";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { UnreadPill, unreadChannelCountLabel } from "@/shared/ui/UnreadPill";

export type UnreadDmPreview = {
  accessibleLabel: string;
  avatarUrl: string | null;
  channelId: string;
  label: string;
};

export function visibleUnreadDmPreviews(dmPreviews: UnreadDmPreview[]) {
  const maxVisibleAvatars = dmPreviews.length > 3 ? 2 : 3;
  return dmPreviews.slice(0, maxVisibleAvatars);
}

export function unreadDmAccessibleLabel({
  count,
  dmPreviews,
  position,
}: {
  count: number;
  dmPreviews: UnreadDmPreview[];
  position: "top" | "bottom";
}) {
  return dmPreviews[0]
    ? `Go to unread direct message from ${dmPreviews[0].accessibleLabel}. ${unreadChannelCountLabel(count, position)}.`
    : unreadChannelCountLabel(count, position);
}

export function preferredUnreadTarget(
  dmPreviews: UnreadDmPreview[],
  nearestChannelId?: string,
) {
  return dmPreviews[0]?.channelId ?? nearestChannelId;
}

export function MoreUnreadButton({
  bottomClassName = "bottom-0",
  count,
  dmPreviews = [],
  label,
  onClick,
  position,
  testId,
}: {
  bottomClassName?: string;
  count: number;
  dmPreviews?: UnreadDmPreview[];
  label?: string;
  onClick: () => void;
  position: "top" | "bottom";
  testId: string;
}) {
  const positionClassName =
    position === "top" ? topChromeInset.top : bottomClassName;
  const visibleDmPreviews = visibleUnreadDmPreviews(dmPreviews);
  const dmOverflowCount = dmPreviews.length - visibleDmPreviews.length;
  const resolvedLabel = label ?? unreadChannelCountLabel(count);
  const accessibleLabel = unreadDmAccessibleLabel({
    count,
    dmPreviews,
    position,
  });

  return (
    <div
      className={`pointer-events-none absolute inset-x-0 z-10 flex justify-center px-2 py-1 ${positionClassName}`}
    >
      <UnreadPill
        accessibleLabel={accessibleLabel}
        className="max-w-full"
        direction={position === "top" ? "up" : "down"}
        emphasis="primary"
        label={resolvedLabel}
        leading={
          visibleDmPreviews.length > 0 ? (
            <span aria-hidden="true" className="flex shrink-0 -space-x-1.5">
              {visibleDmPreviews.map((preview) => (
                <UserAvatar
                  avatarUrl={preview.avatarUrl}
                  className="ring-2 ring-primary"
                  displayName={preview.label}
                  fallbackDelayMs={0}
                  key={preview.channelId}
                  size="xs"
                  testId={`sidebar-unread-dm-avatar-${preview.channelId}`}
                />
              ))}
              {dmOverflowCount > 0 ? (
                <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-primary-foreground px-1 text-3xs font-semibold text-primary ring-2 ring-primary">
                  +{dmOverflowCount}
                </span>
              ) : null}
            </span>
          ) : undefined
        }
        onClick={onClick}
        testId={testId}
      />
    </div>
  );
}
