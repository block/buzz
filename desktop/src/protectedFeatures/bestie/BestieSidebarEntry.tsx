import { Users } from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { parkPlaygroundHost } from "@/features/playground/lib/sessions";
import { leaveLeftNavBuzzTerm } from "@/features/terminal/terminalPanelStore";
import { SidebarMenuButton, SidebarMenuItem } from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";
import { useBestie } from "./useBestie";

export function BestieSidebarEntry() {
  const bestie = useBestie();
  const { goAgents } = useAppNavigation();
  // Nav label stays product name "Bestie"; agent identity lives in the footer.
  const label = "Bestie";

  const handleClick = () => {
    // Same exclusive-surface handoff as pinned websites / primary nav rows:
    // leave left-nav Buzz Term and park playground before navigating.
    parkPlaygroundHost();
    leaveLeftNavBuzzTerm();
    if (!bestie.assignedAgent) {
      void goAgents();
      return;
    }
    void bestie.openConversation().catch((error) => {
      toast.error(
        error instanceof Error
          ? error.message
          : "Couldn’t open Bestie conversation",
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
