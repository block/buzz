import { EllipsisVertical } from "lucide-react";

import {
  channelWebsiteTabLabel,
  type ChannelWebsite,
} from "@/features/channels/lib/channelWebsites";
import { ChannelWebsiteFavicon } from "@/features/channels/ui/ChannelWebsiteFavicon";
import { ChannelWebsiteOverflowMenuItems } from "@/features/channels/ui/ChannelWebsiteOverflowMenuItems";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type ChannelSurface = "chat" | string;

type ChannelWebsiteTabsProps = {
  websites: readonly ChannelWebsite[];
  surface: ChannelSurface;
  onSurfaceChange: (surface: ChannelSurface) => void;
  onRefresh?: () => void;
  onOpenExternal?: () => void;
};

const WEBSITE_TAB_BASE_CLASS =
  "inline-flex h-8 min-w-0 shrink-0 items-center gap-1 rounded-none border-b-2 bg-transparent px-2 text-sm font-medium shadow-none transition-colors";

export function ChannelWebsiteTabs({
  websites,
  surface,
  onSurfaceChange,
  onRefresh,
  onOpenExternal,
}: ChannelWebsiteTabsProps) {
  if (websites.length === 0) return null;

  return (
    <div
      className="flex min-w-0 items-end gap-0.5 self-stretch"
      data-testid="channel-website-tabs"
    >
      {websites.map((website) => {
        const selected = surface === website.id;
        const label = channelWebsiteTabLabel(website);
        return (
          <div
            className={cn(
              "flex min-w-0 items-stretch",
              selected && "border-b-2 border-foreground",
            )}
            key={website.id}
          >
            <button
              aria-label={`${label} website tab`}
              aria-pressed={selected}
              className={cn(
                WEBSITE_TAB_BASE_CLASS,
                selected
                  ? "border-transparent text-foreground"
                  : "border-transparent text-muted-foreground hover:text-foreground",
              )}
              data-testid={`channel-website-tab-${website.id}`}
              onClick={() => onSurfaceChange(website.id)}
              type="button"
            >
              <ChannelWebsiteFavicon label={label} url={website.url} />
              <span className="truncate">{label}</span>
            </button>
            {selected && onRefresh && onOpenExternal ? (
              <DropdownMenu modal={false}>
                <DropdownMenuTrigger asChild>
                  <Button
                    aria-label={`${label} website tab menu`}
                    className="h-8 w-7 shrink-0 rounded-none border-b-2 border-transparent px-0 text-muted-foreground hover:text-foreground"
                    data-testid={`channel-website-tab-menu-${website.id}`}
                    size="icon-xs"
                    type="button"
                    variant="ghost"
                  >
                    <EllipsisVertical className="h-3.5 w-3.5" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-52">
                  <ChannelWebsiteOverflowMenuItems
                    onOpenExternal={onOpenExternal}
                    onRefresh={onRefresh}
                  />
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
