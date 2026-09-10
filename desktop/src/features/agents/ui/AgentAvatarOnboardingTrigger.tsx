import { Plus } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { EmojiAvatarDescriptor } from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { cn } from "@/shared/lib/cn";
import { PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";

export function AgentAvatarOnboardingTrigger({
  assetLabel,
  avatarUrl,
  disabled,
  emojiAvatarPreview,
  hasAvatar,
  isAnimatedPreviewActive,
  isAvatarPending,
  label,
  onPreviewContainerChange,
  squishKey,
  testIdPrefix,
}: {
  assetLabel: string;
  avatarUrl: string | null;
  disabled: boolean;
  emojiAvatarPreview: EmojiAvatarDescriptor | null;
  hasAvatar: boolean;
  isAnimatedPreviewActive: boolean;
  isAvatarPending: boolean;
  label: string;
  onPreviewContainerChange: (container: HTMLDivElement | null) => void;
  squishKey: number;
  testIdPrefix: string;
}) {
  const actionLabel = hasAvatar ? `Change ${assetLabel}` : `Add ${assetLabel}`;

  return (
    <div className="group/avatar-trigger relative h-full w-full">
      <div
        className="pointer-events-none absolute inset-0 z-10 overflow-visible"
        data-testid={`${testIdPrefix}-animated-preview-slot`}
        ref={onPreviewContainerChange}
      />
      {isAnimatedPreviewActive ? null : isAvatarPending ? (
        <div className="grid h-full w-full place-items-center rounded-full border-2 border-dashed border-border bg-background text-foreground shadow-xs">
          <Spinner
            aria-label={`Uploading ${assetLabel}`}
            className="h-4 w-4 border-2"
          />
        </div>
      ) : emojiAvatarPreview ? (
        <div
          aria-label={`${label} ${assetLabel}`}
          className="flex h-full w-full items-center justify-center overflow-hidden rounded-full shadow-xs transition-[background-color] duration-150 ease-out motion-reduce:transition-none"
          data-testid={`${testIdPrefix}-preview`}
          role="img"
          style={{ backgroundColor: emojiAvatarPreview.color }}
        >
          <span
            className={cn(
              "flex h-full w-full items-center justify-center text-6xl leading-none",
              squishKey > 0 && "buzz-avatar-squish",
            )}
            key={squishKey}
          >
            {emojiAvatarPreview.emoji}
          </span>
        </div>
      ) : hasAvatar ? (
        <ProfileAvatar
          avatarUrl={avatarUrl}
          className="h-full w-full text-4xl"
          label={label}
          testId={`${testIdPrefix}-preview`}
        />
      ) : (
        <div
          className="flex h-full w-full items-center justify-center rounded-full border-2 border-dashed border-border bg-background text-foreground shadow-xs transition-[background-color,border-color,color] duration-150 ease-out group-hover/avatar-trigger:border-foreground/30 group-hover/avatar-trigger:bg-[#f5f5f5] motion-reduce:transition-none"
          data-testid={`${testIdPrefix}-empty`}
        >
          <Plus aria-hidden="true" className="size-9" />
        </div>
      )}
      <PopoverTrigger asChild>
        <button
          aria-label={actionLabel}
          className={cn(
            "absolute inset-0 z-20 rounded-full bg-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-60",
            isAnimatedPreviewActive && "pointer-events-none",
          )}
          data-testid={`${testIdPrefix}-open`}
          disabled={disabled || isAvatarPending}
          title={actionLabel}
          type="button"
        />
      </PopoverTrigger>
    </div>
  );
}
