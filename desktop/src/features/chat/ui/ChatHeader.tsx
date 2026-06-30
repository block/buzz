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
import { AnimatedTextSwap } from "@/shared/ui/AnimatedTextSwap";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { useOptionalSidebar } from "@/shared/ui/sidebar";

type ChatHeaderProps = {
  actions?: React.ReactNode;
  animatedTitle?: boolean;
  animatedTitleResetKey?: string;
  belowSystemChrome?: boolean;
  compactTitleStack?: boolean;
  /** Ref to the outer chrome wrapper when `belowSystemChrome` is true. */
  chromeWrapperRef?: React.Ref<HTMLDivElement>;
  title: string;
  description?: string;
  channelType?: ChannelType;
  visibility?: ChannelVisibility;
  leadingContent?: React.ReactNode | false;
  leadingContentContainerClassName?: string;
  leadingContentLayout?: "inline" | "side";
  mode?: "home" | "channel" | "agents" | "workflows" | "pulse" | "projects";
  overlaysContent?: boolean;
  statusBadge?: React.ReactNode;
  /** Render the chrome wrapper without an individual backdrop when a parent supplies shared blur. */
  transparentChrome?: boolean;
  subtitle?: string | null;
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
  animatedTitle = false,
  animatedTitleResetKey,
  belowSystemChrome = false,
  compactTitleStack = false,
  chromeWrapperRef,
  title,
  description,
  channelType,
  visibility,
  leadingContent,
  leadingContentContainerClassName,
  leadingContentLayout = "inline",
  mode = "channel",
  overlaysContent = false,
  statusBadge,
  transparentChrome = false,
  subtitle,
}: ChatHeaderProps) {
  const trimmedDescription = description?.trim() ?? "";
  const trimmedSubtitle = subtitle?.trim() ?? "";
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

  const renderedLeadingContent =
    leadingContent === false
      ? null
      : (leadingContent ?? (
          <ChannelIcon
            channelType={channelType}
            mode={mode}
            visibility={visibility}
          />
        ));

  const header = (
    <header
      className={cn(
        "pointer-events-auto relative z-30 min-w-0 shrink-0 cursor-default select-none bg-transparent px-5 py-2 transition-[margin,padding] duration-200 ease-linear",
        overlaysContent && !belowSystemChrome && "-mb-14",
        clearCollapsedTopChromeControls && "pl-[176px]",
      )}
      data-testid="chat-header"
      data-tauri-drag-region
    >
      <div
        className={cn(
          "flex min-w-0 items-center gap-2.5",
          compactTitleStack ? "min-h-10" : "h-9",
        )}
      >
        {renderedLeadingContent && leadingContentLayout === "side" ? (
          <div className={cn("shrink-0", leadingContentContainerClassName)}>
            {renderedLeadingContent}
          </div>
        ) : null}
        <div className="min-w-0 flex-1">
          <div className="group/title flex min-w-0 items-center gap-[4px] overflow-hidden">
            {renderedLeadingContent && leadingContentLayout === "inline" ? (
              <div className={cn("shrink-0", leadingContentContainerClassName)}>
                {renderedLeadingContent}
              </div>
            ) : null}
            <h1
              className={cn(
                "min-w-0 truncate text-base font-semibold tracking-tight",
                compactTitleStack ? "leading-5" : "translate-y-px leading-6",
              )}
              data-testid="chat-title"
              title={trimmedDescription || undefined}
            >
              {animatedTitle ? (
                <AnimatedTextSwap
                  className="max-w-full overflow-hidden text-ellipsis"
                  key={animatedTitleResetKey}
                  value={title}
                />
              ) : (
                title
              )}
            </h1>
            <Button
              aria-label={`Copy channel name: ${title}`}
              className={cn(
                "shrink-0 opacity-0 text-muted-foreground transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/title:opacity-100",
                compactTitleStack ? "h-5 w-5 [&_svg]:size-3" : "h-6 w-6",
              )}
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
          {trimmedSubtitle ? (
            <p
              className={cn(
                "truncate text-xs text-muted-foreground",
                compactTitleStack ? "-mt-0.5 leading-4" : "leading-4",
              )}
            >
              {trimmedSubtitle}
            </p>
          ) : null}
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
