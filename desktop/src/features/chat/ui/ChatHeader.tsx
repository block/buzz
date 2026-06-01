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
import { createPortal } from "react-dom";

import type { ChannelType, ChannelVisibility } from "@/shared/api/types";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import { cn } from "@/shared/lib/cn";

type ChatHeaderProps = {
  actions?: React.ReactNode;
  actionsPlacement?: "inline" | "top-right";
  belowSystemChrome?: boolean;
  density?: "default" | "compact";
  title: string;
  description?: string;
  channelType?: ChannelType;
  visibility?: ChannelVisibility;
  mode?: "home" | "channel" | "agents" | "workflows" | "pulse" | "projects";
  overlaysContent?: boolean;
  statusBadge?: React.ReactNode;
};

const HEADER_ICON_CLASS = "h-[14px] w-[14px] text-muted-foreground";
const CHANNEL_HASH_ICON_CLASS = "h-[14px] w-[14px] translate-y-px";

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
  actionsPlacement = "inline",
  belowSystemChrome = false,
  density = "default",
  title,
  description,
  channelType,
  visibility,
  mode = "channel",
  overlaysContent = false,
  statusBadge,
}: ChatHeaderProps) {
  const trimmedDescription = description?.trim() ?? "";
  const topRightActions = (
    <div className="fixed right-3 top-[9px] z-[70] flex shrink-0 items-center gap-1">
      <UpdateIndicator />
      {actions ? <div className="shrink-0">{actions}</div> : null}
    </div>
  );

  const header = (
    <header
      className={cn(
        "relative z-30 flex min-w-0 shrink-0 cursor-default select-none items-center gap-[10px] bg-transparent pl-[16px] pr-[8px] transition-[margin,padding] duration-200 ease-linear sm:pl-[24px] sm:pr-[12px]",
        density === "compact"
          ? "min-h-[32px] py-[4px]"
          : "min-h-[44px] py-[6px]",
        overlaysContent && !belowSystemChrome && "-mb-[44px]",
      )}
      data-testid="chat-header"
      data-tauri-drag-region
    >
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-center gap-[4px]">
          <ChannelIcon
            channelType={channelType}
            mode={mode}
            visibility={visibility}
          />
          <h1
            className="min-w-0 translate-y-px truncate text-sm font-semibold leading-5 tracking-tight"
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

      {actionsPlacement === "top-right" ? (
        typeof document === "undefined" ? null : (
          createPortal(topRightActions, document.body)
        )
      ) : (
        <div className="flex shrink-0 items-center gap-1">
          <UpdateIndicator />
          {actions ? <div className="shrink-0">{actions}</div> : null}
        </div>
      )}
    </header>
  );

  if (!belowSystemChrome) {
    return header;
  }

  return (
    <div className="relative z-30 h-[76px] -mb-[76px] bg-background/80 pt-[42px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55">
      {header}
    </div>
  );
}
