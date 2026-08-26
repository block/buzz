import { topChromeInset } from "@/shared/layout/chromeLayout";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { UnreadPill, unreadCountLabel } from "@/shared/ui/UnreadPill";

export type UnreadDmPreview = {
  accessibleLabel: string;
  avatarUrl: string | null;
  channelId: string;
  label: string;
};

export function visibleUnreadDmPreviews(dmPreviews: UnreadDmPreview[]) {
  return dmPreviews.slice(0, 3);
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
  const direction = position === "top" ? "above" : "below";
  return dmPreviews[0]
    ? `Go to unread direct message from ${dmPreviews[0].accessibleLabel}. ${unreadCountLabel(count)} ${direction}.`
    : `${unreadCountLabel(count)} ${direction}`;
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
  const resolvedLabel = label ?? unreadCountLabel(count);
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
            <span
              aria-hidden="true"
              className="flex shrink-0 items-center gap-1.5"
            >
              <span className="flex -space-x-1.5">
                {visibleDmPreviews.map((preview, index) => (
                  <span
                    className="relative"
                    key={preview.channelId}
                    style={{ zIndex: visibleDmPreviews.length - index }}
                  >
                    <UserAvatar
                      avatarUrl={preview.avatarUrl}
                      className="ring-2 ring-primary"
                      displayName={preview.label}
                      fallbackDelayMs={0}
                      size="xs"
                      testId={`sidebar-unread-dm-avatar-${preview.channelId}`}
                    />
                  </span>
                ))}
              </span>
              <span>·</span>
            </span>
          ) : undefined
        }
        onClick={onClick}
        testId={testId}
      />
    </div>
  );
}
