import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";

type UserAvatarSize = "xs" | "sm" | "md";

const sizeClasses: Record<UserAvatarSize, string> = {
  xs: "h-5 w-5 text-[8px]",
  sm: "h-6 w-6 text-[9px]",
  md: "h-10 w-10 text-xs",
};

type UserAvatarProps = {
  avatarUrl: string | null;
  displayName: string;
  size?: UserAvatarSize;
  accent?: boolean;
  className?: string;
  testId?: string;
};

export function UserAvatar({
  avatarUrl,
  displayName,
  size = "md",
  accent = false,
  className,
  testId,
}: UserAvatarProps) {
  const initials = getInitials(displayName);

  return (
    <Avatar className={cn(sizeClasses[size], "shadow-xs", className)}>
      {avatarUrl ? (
        <AvatarImage
          alt={`${displayName} avatar`}
          className="bg-secondary object-cover"
          data-testid={testId ? `${testId}-image` : undefined}
          referrerPolicy="no-referrer"
          src={rewriteRelayUrl(avatarUrl)}
        />
      ) : null}
      <AvatarFallback
        className={cn(
          "font-semibold",
          accent
            ? "bg-primary text-primary-foreground"
            : "bg-secondary text-secondary-foreground",
        )}
        data-testid={testId ? `${testId}-fallback` : undefined}
        delayMs={200}
      >
        {initials}
      </AvatarFallback>
    </Avatar>
  );
}
