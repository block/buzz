import type * as React from "react";
import { AppWindow } from "lucide-react";

import type { SurfaceTabTarget } from "@/app/surfaceTabs/surfaceTabModel";
import { useOptionalSurfaceTabs } from "@/app/surfaceTabs/SurfaceTabsProvider";
import { isPhoneLayout } from "@/shared/lib/phoneLayout";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";

import {
  ContextMenuIconSlot,
  deferMenuAction,
} from "@/features/sidebar/ui/sidebarMenuHelpers";

/**
 * "Add as Tab" for a left-panel row. Hidden when the tab strip is not mounted.
 */
export function AddSurfaceTabMenuItem({
  divided = false,
  label,
  target,
}: {
  /** Draw a separator under the item when the menu has more rows. */
  divided?: boolean;
  label: string;
  target: SurfaceTabTarget;
}) {
  const tabs = useOptionalSurfaceTabs();
  if (!tabs) return null;

  return (
    <>
      <ContextMenuItem
        data-testid="add-surface-tab"
        onSelect={() => {
          deferMenuAction(() => {
            tabs.addTab({ label, target });
          });
        }}
      >
        <ContextMenuIconSlot>
          <AppWindow className="h-4 w-4" />
        </ContextMenuIconSlot>
        <span>{isPhoneLayout() ? "Open in new tab" : "Add as Tab"}</span>
      </ContextMenuItem>
      {divided ? <ContextMenuSeparator /> : null}
    </>
  );
}

/** Right-click menu with a single Add as Tab action. */
export function SidebarSurfaceTabMenu({
  children,
  label,
  target,
}: {
  children: React.ReactElement;
  label: string;
  target: SurfaceTabTarget;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <AddSurfaceTabMenuItem label={label} target={target} />
      </ContextMenuContent>
    </ContextMenu>
  );
}
