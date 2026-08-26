import { ExternalLink, RotateCw } from "lucide-react";

import {
  DropdownMenuItem,
  DropdownMenuSeparator,
} from "@/shared/ui/dropdown-menu";

type ChannelWebsiteOverflowMenuItemsProps = {
  onRefresh: () => void;
  onOpenExternal: () => void;
  showSeparator?: boolean;
};

export function ChannelWebsiteOverflowMenuItems({
  onRefresh,
  onOpenExternal,
  showSeparator = false,
}: ChannelWebsiteOverflowMenuItemsProps) {
  return (
    <>
      <DropdownMenuItem
        data-testid="channel-website-refresh"
        onSelect={onRefresh}
      >
        <RotateCw />
        Refresh
      </DropdownMenuItem>
      <DropdownMenuItem
        data-testid="channel-website-open-external"
        onSelect={onOpenExternal}
      >
        <ExternalLink />
        Open in external browser
      </DropdownMenuItem>
      {showSeparator ? <DropdownMenuSeparator /> : null}
    </>
  );
}
