import {
  Activity,
  Bot,
  CircleDot,
  Copy,
  FileText,
  FolderGit2,
  Hash,
  House,
  Lock,
  Zap,
} from "lucide-react";
import type * as React from "react";
import { toast } from "sonner";

import type { ChannelType, ChannelVisibility } from "@/shared/api/types";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

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
  const sidebar = useOptionalSidebar();
  const clearCollapsedTopChromeControls =
    belowSystemChrome && sidebar?.state === "collapsed" && !sidebar.isMobile;

  async function handleCopyTitle() {
    const value = title.trim();
    if (!value) return;

    try {
      await navigator.clipboard.writeText(value);
      toast.success("Channel name copied");
    } catch {
      toast.error("Failed to copy channel name");
    }
  }

  const header = (
    <header
      className={cn(
        "pointer-events-auto relative z-30 flex min-w-0 shrink-0 cursor-default select-none items-center gap-2.5 bg-transparent px-5 transition-[margin,padding] duration-200 ease-linear",
        density === "compact"
          ? belowSystemChrome
            ? "min-h-8 py-1.5"
            : "min-h-8 py-0"
          : "min-h-11 py-1.5",
        overlaysContent && !belowSystemChrome && "-mb-11",
        clearCollapsedTopChromeControls && "pl-[176px]",
      )}
      data-testid="chat-header"
      data-tauri-drag-region
    >
      <div className="min-w-0 flex-1">
        <div className="group/title flex min-w-0 items-center gap-[4px] overflow-hidden">
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
            className="min-w-0 translate-y-px truncate text-base font-semibold leading-6 tracking-tight"
            data-testid="chat-title"
            title={trimmedDescription || undefined}
          >
            {title}
          </h1>
          <Button
            aria-label={`Copy channel name: ${title}`}
            className="h-6 w-6 shrink-0 opacity-0 text-muted-foreground transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/title:opacity-100"
            onClick={() => void handleCopyTitle()}
            size="icon-xs"
            title="Copy channel name"
            type="button"
            variant="ghost"
          >
            <Copy className="h-3.5 w-3.5" />
          </Button>
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
        channelChrome.negativeMargin,
      )}
    >
      {header}
    </div>
  );
}
