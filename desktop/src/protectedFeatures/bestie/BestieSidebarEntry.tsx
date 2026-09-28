import { Users } from "lucide-react";
import { toast } from "sonner";

import { parkPlaygroundHost } from "@/features/playground/lib/sessions";
import { leaveLeftNavBuzzTerm } from "@/features/terminal/terminalPanelStore";
import { BESTIE_POPOVER_SHORTCUT_EVENT } from "@/shared/lib/keyboard-shortcuts";
import { SidebarMenuButton, SidebarMenuItem } from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";
import { useBestie } from "./useBestie";

export function BestieSidebarEntry() {
  const bestie = useBestie();
  // Nav label is product name "Assistant"; agent identity lives in the footer.
  const label = "Assistant";

  const handleClick = () => {
    // Same exclusive-surface handoff as pinned websites / primary nav rows:
    // leave left-nav Buzz Term and park playground before navigating.
    parkPlaygroundHost();
    leaveLeftNavBuzzTerm();
    if (!bestie.assignedAgent) {
      // Stay on Assistant: open empty popover (+ inside → Agents). Do not
      // redirect to the Agents page.
      window.dispatchEvent(
        new CustomEvent(BESTIE_POPOVER_SHORTCUT_EVENT, {
          detail: { open: true },
        }),
      );
      return;
    }
    void bestie.openConversation().catch((error) => {
      toast.error(
        error instanceof Error
          ? error.message
          : "Couldn’t open Assistant conversation",
      );
    });
  };

  return (
    <SidebarMenuItem data-testid="bestie-sidebar-entry">
      <SidebarMenuButton
        disabled={bestie.isOpening}
        onClick={handleClick}
        tooltip={label}
        type="button"
      >
        <Users className="h-4 w-4" />
        <SidebarMenuLabel>{label}</SidebarMenuLabel>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
