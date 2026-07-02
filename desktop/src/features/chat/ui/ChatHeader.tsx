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
  MessageCircle,
  Zap,
} from "lucide-react";
import type * as React from "react";
import { toast } from "sonner";

import type { ChannelType, ChannelVisibility } from "@/shared/api/types";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";

type ChatHeaderProps = {
  actions?: React.ReactNode;
  belowSystemChrome?: boolean;
  /** Ref to the outer chrome wrapper when `belowSystemChrome` is true. */
  chromeWrapperRef?: React.Ref<HTMLDivElement>;
  title: string;
  description?: string;
  channelType?: ChannelType;
  visibility?: ChannelVisibility;
  leadingContent?: React.ReactNode;
  mode?:
    | "home"
    | "channel"
    | "chats"
    | "agents"
    | "workflows"
    | "pulse"
    | "projects";
  overlaysContent?: boolean;
  statusBadge?: React.ReactNode;
  /** Render the chrome wrapper without an individual backdrop when a parent supplies shared blur. */
  transparentChrome?: boolean;
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
  mode?:
    | "home"
    | "channel"
    | "chats"
    | "agents"
    | "workflows"
    | "pulse"
    | "projects";
}) {
  if (mode === "home") {
    return <House className={HEADER_ICON_CLASS} />;
  }

  if (mode === "agents") {
    return <Bot className={HEADER_ICON_CLASS} />;
  }

  if (mode === "chats") {
    return <MessageCircle className={HEADER_ICON_CLASS} />;
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
  title,
  description,
  channelType,
  visibility,
  leadingContent,
  mode = "channel",
  overlaysContent = false,
  statusBadge,
  transparentChrome = false,
}: ChatHeaderProps) {
  const trimmedDescription = description?.trim() ?? "";

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
        "pointer-events-auto relative z-30 min-w-0 shrink-0 cursor-default select-none px-5 py-2 transition-[margin,padding] duration-200 ease-linear",
        !belowSystemChrome && transparentChrome
          ? "bg-background/75 backdrop-blur-md supports-backdrop-filter:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/35"
          : "bg-transparent",
        overlaysContent && !belowSystemChrome && "-mb-14",
      )}
      data-testid="chat-header"
      data-tauri-drag-region
    >
      <div className="flex h-9 min-w-0 items-center gap-2.5">
        <div className="min-w-0 flex-1">
          <div className="group/title flex min-w-0 items-center gap-[4px] overflow-hidden">
            <div className="flex shrink-0 items-center">
              {leadingContent ?? (
                <ChannelIcon
                  channelType={channelType}
                  mode={mode}
                  visibility={visibility}
                />
              )}
            </div>
            <h1
              className={cn(
                "min-w-0 truncate text-base font-semibold leading-6 tracking-tight",
                channelType !== "dm" && "translate-y-px",
              )}
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
        "pointer-events-none relative z-40 overflow-visible rounded-tl-xl",
        transparentChrome
          ? "bg-transparent"
          : "bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
        channelChrome.negativeMargin,
      )}
    >
      {header}
    </div>
  );
}
