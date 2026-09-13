import { Plus } from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { SidebarMenuButton, SidebarMenuItem } from "@/shared/ui/sidebar";
import { BestieAgentLockup } from "./BestiePopover";
import { useBestie } from "./useBestie";

export function BestieSidebarEntry() {
  const bestie = useBestie();
  const { goAgents } = useAppNavigation();

  const handleClick = () => {
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
        tooltip="Bestie"
        type="button"
      >
        {bestie.assignedAgent ? (
          <BestieAgentLockup
            agent={bestie.assignedAgent}
            compact
            presenceStatus={bestie.presenceStatus ?? "offline"}
          />
        ) : (
          <>
            <Plus className="h-4 w-4" />
            <span>Bestie</span>
          </>
        )}
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
