import { FileText, Hash, Lock, MessageSquare } from "lucide-react";

import type { ChannelType, ChannelVisibility } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Avatar, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";

const sizeClasses = {
  sm: "h-6 w-6",
  md: "h-8 w-8",
  lg: "h-20 w-20",
} as const;

const iconSizeClasses = {
  sm: "h-4 w-4",
  md: "h-4 w-4",
  lg: "h-8 w-8",
} as const;

type ChannelAvatarSize = keyof typeof sizeClasses;

type ChannelAvatarProps = {
  avatarUrl?: string | null;
  channelType: ChannelType;
  className?: string;
  fallbackClassName?: string;
  imageClassName?: string;
  name: string;
  size?: ChannelAvatarSize;
  testId?: string;
  visibility: ChannelVisibility;
};

function ChannelFallbackIcon({
  channelType,
  className,
  visibility,
}: {
  channelType: ChannelType;
  className?: string;
  visibility: ChannelVisibility;
}) {
  if (channelType === "dm") {
    return <MessageSquare className={className} />;
  }

  if (visibility === "private") {
    return <Lock className={className} />;
  }

  if (channelType === "forum") {
    return <FileText className={className} />;
  }

  return <Hash className={className} />;
}

export function ChannelAvatar({
  avatarUrl,
  channelType,
  className,
  fallbackClassName,
  imageClassName,
  name,
  size = "md",
  testId,
  visibility,
}: ChannelAvatarProps) {
  const src = avatarUrl ? rewriteRelayUrl(avatarUrl) : null;

  return (
    <Avatar
      className={cn(sizeClasses[size], "shadow-xs", className)}
      data-testid={testId}
    >
      {src ? (
        <AvatarImage
          alt={`${name} channel icon`}
          className={cn("bg-secondary object-cover", imageClassName)}
          data-testid={testId ? `${testId}-image` : undefined}
          referrerPolicy="no-referrer"
          src={src}
        />
      ) : null}
      <AvatarFallback
        className={cn(
          "bg-secondary text-secondary-foreground",
          fallbackClassName,
        )}
        data-testid={testId ? `${testId}-fallback` : undefined}
      >
        <ChannelFallbackIcon
          channelType={channelType}
          className={iconSizeClasses[size]}
          visibility={visibility}
        />
      </AvatarFallback>
    </Avatar>
  );
}
