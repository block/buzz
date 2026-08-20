import { UserAvatar } from "@/shared/ui/UserAvatar";

export type AvatarStackItem = {
  id: string;
  avatarUrl: string | null;
  displayName: string;
};

export function AvatarStack({
  items,
  limit = items.length,
  testId = "avatar-stack-item",
}: {
  items: readonly AvatarStackItem[];
  limit?: number;
  testId?: string;
}) {
  const visible = items.slice(0, limit);
  const overflow = items.length - visible.length;

  return (
    <span className="flex h-6 shrink-0 items-center" data-testid="avatar-stack">
      {visible.map((item, index) => (
        <span
          className={index > 0 ? "-ml-1" : ""}
          data-testid={testId}
          key={item.id}
          style={{
            zIndex: index + 1,
            ...(index < visible.length - 1 && {
              mask: "radial-gradient(circle 14px at calc(100% + 6px) 50%, transparent 99%, #fff 100%)",
              WebkitMask:
                "radial-gradient(circle 14px at calc(100% + 6px) 50%, transparent 99%, #fff 100%)",
            }),
          }}
        >
          <UserAvatar
            avatarUrl={item.avatarUrl}
            className="h-6 w-6 text-2xs"
            displayName={item.displayName}
            size="sm"
          />
        </span>
      ))}
      {overflow > 0 ? (
        <span className="relative z-10 ml-0.5 inline-grid h-[1.375rem] min-w-[1.6875rem] place-items-center rounded-full border border-border bg-background px-1 text-3xs font-semibold text-muted-foreground">
          +{overflow}
        </span>
      ) : null}
    </span>
  );
}
