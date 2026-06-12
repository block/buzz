import * as React from "react";
import { UserRound } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";

type ProfileAvatarProps = {
  avatarUrl: string | null;
  avatarDataUrl?: string | null;
  label: string;
  className?: string;
  iconClassName?: string;
  plain?: boolean;
  testId?: string;
};

export function ProfileAvatar({
  avatarUrl,
  avatarDataUrl,
  label,
  className,
  iconClassName,
  plain = false,
  testId,
}: ProfileAvatarProps) {
  const initials = getInitials(label);

  // Compute the live (proxied) source and reset failure state when the URL changes.
  const liveSrc = avatarUrl ? rewriteRelayUrl(avatarUrl) : null;
  const [liveFailed, setLiveFailed] = React.useState(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: avatarUrl is the trigger — reset liveFailed when the URL changes even though the effect body doesn't reference it directly.
  React.useEffect(() => {
    setLiveFailed(false);
  }, [avatarUrl]);

  // When the relay is unreachable the proxied avatar URL 404s/times out; fall
  // back to the locally cached data URL instead of dropping to initials.
  const src = liveFailed
    ? (avatarDataUrl ?? undefined)
    : (liveSrc ?? avatarDataUrl ?? undefined);

  return (
    <Avatar
      className={cn(
        "shrink-0 text-primary shadow-xs",
        plain ? "bg-transparent shadow-none" : "bg-primary/20",
        className,
      )}
      data-testid={testId}
    >
      {src !== undefined ? (
        <AvatarImage
          alt={`${label} avatar`}
          className="object-cover"
          onLoadingStatusChange={(status) => {
            if (status === "error") setLiveFailed(true);
          }}
          referrerPolicy="no-referrer"
          src={src}
        />
      ) : null}
      <AvatarFallback
        className={cn(
          "font-semibold text-primary",
          plain ? "bg-transparent" : "bg-primary/20",
        )}
        delayMs={src === undefined ? undefined : 200}
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
