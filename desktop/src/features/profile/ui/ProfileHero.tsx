import { Archive, ChevronDown, ChevronUp } from "lucide-react";
import * as React from "react";

import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type {
  ProfileSummaryData,
  ProfileSummaryUserStatus,
} from "@/features/profile/ui/profileSummaryTypes";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { cn } from "@/shared/lib/cn";

export function ProfileHero({
  displayName,
  isArchived,
  isBot,
  presenceStatus,
  profile,
  userStatus,
}: {
  displayName: string;
  isArchived: boolean | undefined;
  isBot: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ProfileSummaryData;
  userStatus: ProfileSummaryUserStatus;
}) {
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <div className="relative">
        <ProfileAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-20 w-20 text-xl"
          iconClassName="h-8 w-8"
          label={displayName}
          plain
          testId="user-profile-avatar"
        />
        {presenceStatus ? (
          <span
            aria-label={getPresenceLabel(presenceStatus)}
            className="absolute bottom-0 right-0 flex h-6 w-6 items-center justify-center rounded-full bg-background"
            data-testid="user-profile-presence-badge"
            role="img"
          >
            <PresenceDot className="h-3.5 w-3.5" status={presenceStatus} />
          </span>
        ) : null}
      </div>

      <div className="flex flex-col items-center gap-1">
        <div className="flex items-center justify-center gap-2">
          <h3 className="text-xl font-semibold tracking-tight">
            {displayName}
          </h3>
          {isBot ? (
            <BotIdenticon
              className="shrink-0 rounded"
              size={20}
              value={displayName}
            />
          ) : null}
        </div>

        {isArchived === true ? (
          <span
            className="mt-1 inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300"
            data-testid="user-profile-archived-flair"
            title="This identity is archived on this relay. Historical events remain attributed to it."
          >
            <Archive className="h-3 w-3" />
            Archived on this relay
          </span>
        ) : null}

        {profile?.about?.trim() ? (
          <ProfileHeroDescription
            about={profile.about.trim()}
            key={profile.about.trim()}
          />
        ) : null}

        {profile?.nip05Handle ? (
          <p className="text-sm text-muted-foreground">{profile.nip05Handle}</p>
        ) : null}

        {userStatus ? (
          <p className="text-sm text-muted-foreground">
            {userStatus.emoji ? (
              <StatusEmoji
                className="mr-1 inline h-3.5 w-3.5"
                value={userStatus.emoji}
              />
            ) : null}
            {userStatus.text}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function ProfileHeroDescription({ about }: { about: string }) {
  const [expanded, setExpanded] = React.useState(false);
  const [isTruncated, setIsTruncated] = React.useState(false);
  const textRef = React.useRef<HTMLParagraphElement>(null);

  const measureTruncation = React.useCallback(() => {
    const element = textRef.current;
    if (!element || expanded) {
      return;
    }
    setIsTruncated(element.scrollHeight > element.clientHeight + 1);
  }, [expanded]);

  React.useLayoutEffect(() => {
    measureTruncation();
  }, [measureTruncation]);

  React.useEffect(() => {
    const element = textRef.current;
    if (!element) {
      return;
    }

    const observer = new ResizeObserver(() => {
      measureTruncation();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [measureTruncation]);

  const toggleClassName =
    "inline-flex items-center gap-0.5 text-xs font-medium text-muted-foreground opacity-60 transition-opacity hover:text-foreground hover:opacity-100";

  return (
    <div className="flex w-full flex-col items-center gap-0.5">
      <div className="w-fit max-w-full px-2">
        <p
          className={cn(
            "text-center whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground",
            !expanded && "line-clamp-3",
          )}
          data-testid="user-profile-description"
          ref={textRef}
        >
          {about}
        </p>
      </div>
      {!expanded && isTruncated ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(true)}
          type="button"
        >
          more
          <ChevronDown className="h-3 w-3" />
        </button>
      ) : null}
      {expanded ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(false)}
          type="button"
        >
          less
          <ChevronUp className="h-3 w-3" />
        </button>
      ) : null}
    </div>
  );
}
