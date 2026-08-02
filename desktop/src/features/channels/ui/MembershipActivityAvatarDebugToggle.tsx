import * as React from "react";
import { createPortal } from "react-dom";
import { useFeatureToggle } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";

export function MembershipActivityAvatarDebugToggle({
  channelType,
}: {
  channelType: string | null | undefined;
}) {
  const [host, setHost] = React.useState<HTMLElement | null>(null);
  const [showAvatarStacks, setShowAvatarStacks] = useFeatureToggle(
    "membershipActivityAvatarStacks",
  );
  const [useInlineAvatarLayout, setUseInlineAvatarLayout] = useFeatureToggle(
    "membershipActivityAvatarInlineLayout",
  );

  React.useLayoutEffect(() => {
    setHost(
      document.querySelector<HTMLElement>("[data-testid='app-top-chrome']"),
    );
  }, []);

  if (channelType === undefined || channelType === "forum" || !host)
    return null;

  return createPortal(
    <div
      className="absolute right-4 top-1/2 z-10 flex -translate-y-1/2 items-center gap-2 rounded-full border border-sidebar-border/60 bg-sidebar/45 px-2.5 py-1 text-2xs font-medium text-sidebar-foreground shadow-sm backdrop-blur-md"
      data-testid="membership-activity-avatar-debug-toggle"
    >
      <span>Activity avatars</span>
      <Switch
        aria-label="Show membership activity avatars"
        checked={showAvatarStacks}
        className="h-4 w-7 [&>span]:h-3 [&>span]:w-3 data-[state=checked]:[&>span]:translate-x-3"
        onCheckedChange={setShowAvatarStacks}
      />
      <span aria-hidden className="h-3 w-px bg-sidebar-border/70" />
      <div
        className="flex rounded-md bg-sidebar-accent/50 p-0.5"
        data-testid="membership-activity-avatar-layout-toggle"
      >
        <button
          aria-pressed={!useInlineAvatarLayout}
          className={
            useInlineAvatarLayout
              ? "rounded px-1.5 py-0.5 text-sidebar-foreground/65"
              : "rounded bg-sidebar px-1.5 py-0.5 text-sidebar-foreground shadow-xs"
          }
          disabled={!showAvatarStacks}
          onClick={() => setUseInlineAvatarLayout(false)}
          type="button"
        >
          Top
        </button>
        <button
          aria-pressed={useInlineAvatarLayout}
          className={
            useInlineAvatarLayout
              ? "rounded bg-sidebar px-1.5 py-0.5 text-sidebar-foreground shadow-xs"
              : "rounded px-1.5 py-0.5 text-sidebar-foreground/65"
          }
          disabled={!showAvatarStacks}
          onClick={() => setUseInlineAvatarLayout(true)}
          type="button"
        >
          Inline
        </button>
      </div>
    </div>,
    host,
  );
}
