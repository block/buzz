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
import { useSidebar } from "@/shared/ui/sidebar";

type ChatHeaderProps = {
  actions?: React.ReactNode;
  title: string;
  description?: string;
  channelType?: ChannelType;
  visibility?: ChannelVisibility;
  mode?: "home" | "channel" | "agents" | "workflows" | "pulse" | "projects";
  overlaysContent?: boolean;
  statusBadge?: React.ReactNode;
};

const HEADER_ICON_CLASS = "h-[14px] w-[14px] text-muted-foreground";

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

  return <Hash className={HEADER_ICON_CLASS} />;
}

export function ChatHeader({
  actions,
  title,
  description,
  channelType,
  visibility,
  mode = "channel",
  overlaysContent = false,
  statusBadge,
}: ChatHeaderProps) {
  const trimmedDescription = description?.trim() ?? "";
  const { state: sidebarState } = useSidebar();
  const reserveGlobalControls = sidebarState === "collapsed";

  return (
    <header
      className={cn(
        "relative z-30 flex min-h-[44px] min-w-0 shrink-0 cursor-default select-none items-center gap-[10px] bg-background/70 py-[6px] pl-[16px] pr-[8px] backdrop-blur-xl transition-[margin,padding] duration-200 ease-linear supports-[backdrop-filter]:bg-background/55 sm:pl-[24px] sm:pr-[12px]",
        overlaysContent && "-mb-[44px]",
        reserveGlobalControls && "md:pl-[160px]",
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
            className="min-w-0 truncate text-sm font-semibold leading-5 tracking-tight"
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
}
