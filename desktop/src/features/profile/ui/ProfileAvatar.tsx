import { UserRound } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";

type ProfileAvatarProps = {
  avatarUrl: string | null;
  label: string;
  className?: string;
  iconClassName?: string;
  plain?: boolean;
  testId?: string;
};

export function ProfileAvatar({
  avatarUrl,
  label,
  className,
  iconClassName,
  plain = false,
  testId,
}: ProfileAvatarProps) {
  const initials = getInitials(label);

  return (
    <Avatar
      className={cn(
        "shrink-0 text-primary shadow-xs",
        plain ? "bg-transparent shadow-none" : "bg-primary/20",
        className,
      )}
      data-testid={testId}
    >
      {avatarUrl ? (
        <AvatarImage
          alt={`${label} avatar`}
          className="object-cover"
          referrerPolicy="no-referrer"
          src={rewriteRelayUrl(avatarUrl)}
        />
      ) : null}
      <AvatarFallback
        className={cn(
          "font-semibold text-primary",
          plain ? "bg-transparent" : "bg-primary/20",
        )}
        delayMs={200}
      >
        {initials.length > 0 ? (
          initials
        ) : (
          <UserRound className={iconClassName} />
        )}
      </AvatarFallback>
    </Avatar>
  );
}
