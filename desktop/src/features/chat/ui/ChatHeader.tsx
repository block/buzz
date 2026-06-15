import {
  Activity,
  Bot,
  CircleDot,
  FileText,
  FolderGit2,
  Hash,
  House,
  Lock,
  Zap,
} from "lucide-react";
import type * as React from "react";

import type { ChannelType, ChannelVisibility } from "@/shared/api/types";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import { cn } from "@/shared/lib/cn";
import { channelChrome, topChromeInset } from "@/shared/layout/chromeLayout";

type ChatHeaderProps = {
  actions?: React.ReactNode;
  belowSystemChrome?: boolean;
  /** Ref to the outer chrome wrapper when `belowSystemChrome` is true. */
  chromeWrapperRef?: React.Ref<HTMLDivElement>;
  density?: "default" | "compact";
  title: string;
  description?: string;
  channelType?: ChannelType;
  visibility?: ChannelVisibility;
  leadingContent?: React.ReactNode;
  mode?: "home" | "channel" | "agents" | "workflows" | "pulse" | "projects";
  overlaysContent?: boolean;
  statusBadge?: React.ReactNode;
};

const HEADER_ICON_CLASS = "h-4 w-4 text-muted-foreground";
const CHANNEL_HASH_ICON_CLASS = "h-4 w-4 translate-y-px";

function ChannelIcon({
  channelType,
  visibility,
  mode = "channel",
}: {
  channelType?: ChannelType;
  visibility?: ChannelVisibility;
  mode?: "home" | "channel" | "agents" | "workflows" | "pulse" | "projects";
}) {
  if (mode === "home") {
    return <House className={HEADER_ICON_CLASS} />;
  }

  if (mode === "agents") {
    return <Bot className={HEADER_ICON_CLASS} />;
  }

  if (mode === "workflows") {
    return <Zap className={HEADER_ICON_CLASS} />;
  }

  if (mode === "pulse") {
    return <Activity className={HEADER_ICON_CLASS} />;
  }

  if (mode === "projects") {
    return <FolderGit2 className={HEADER_ICON_CLASS} />;
  }

  if (channelType === "dm") {
    return <CircleDot className={HEADER_ICON_CLASS} />;
  }

  if (visibility === "private") {
    return <Lock className={HEADER_ICON_CLASS} />;
  }

  if (channelType === "forum") {
    return <FileText className={HEADER_ICON_CLASS} />;
  }

  return <Hash className={CHANNEL_HASH_ICON_CLASS} color="gray" />;
}

export function ChatHeader({
  actions,
  belowSystemChrome = false,
  chromeWrapperRef,
  density = "default",
  title,
  description,
  channelType,
  visibility,
  leadingContent,
  mode = "channel",
  overlaysContent = false,
  statusBadge,
}: ChatHeaderProps) {
  const trimmedDescription = description?.trim() ?? "";

  const header = (
    <header
      className={cn(
        "pointer-events-auto relative z-30 flex min-w-0 shrink-0 cursor-default select-none items-center gap-2.5 bg-transparent px-4 transition-[margin,padding] duration-200 ease-linear sm:px-6",
        density === "compact"
          ? belowSystemChrome
            ? "min-h-8 py-1.5"
            : "min-h-8 py-0"
          : "min-h-11 py-1.5",
        overlaysContent && !belowSystemChrome && "-mb-11",
      )}
      data-testid="chat-header"
      data-tauri-drag-region
    >
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-[4px] overflow-hidden">
          <div className="shrink-0">
            {leadingContent ?? (
              <ChannelIcon
                channelType={channelType}
                mode={mode}
                visibility={visibility}
              />
            )}
          </div>
          <h1
            className="min-w-0 flex-1 translate-y-px truncate text-base font-semibold leading-6 tracking-tight"
            data-testid="chat-title"
            title={trimmedDescription || undefined}
          >
            {title}
          </h1>
          {statusBadge ? (
            <div className="flex shrink-0 flex-wrap items-center gap-1">
              {statusBadge}
            </div>
          ) : null}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1">
        <UpdateIndicator />
        {actions ? <div className="shrink-0">{actions}</div> : null}
      </div>
    </header>
  );

  if (!belowSystemChrome) {
    return header;
  }

  return (
    <div
      ref={chromeWrapperRef}
      className={cn(
        "pointer-events-none relative z-30 bg-background/80 backdrop-blur-md after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-border/35 after:content-[''] supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
        topChromeInset.padding,
        channelChrome.negativeMargin,
      )}
    >
      {header}
    </div>
  );
}
