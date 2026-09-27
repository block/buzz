import { AppWindow } from "lucide-react";
import { useSyncExternalStore } from "react";

import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";

import {
  getPlaygroundStore,
  showPlaygroundSession,
  subscribePlayground,
} from "../lib/sessions";

export function PlaygroundSection() {
  const { sessions, overlaySid } = useSyncExternalStore(
    subscribePlayground,
    getPlaygroundStore,
    getPlaygroundStore,
  );
  const rows = [...sessions.values()];
  if (rows.length === 0) return null;

  return (
    <SidebarGroup data-testid="playgrounds-section">
      <SidebarGroupLabel data-testid="playgrounds-section-label">
        Playgrounds
      </SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {rows.map((session) => (
            <SidebarMenuItem key={session.sid}>
              <SidebarMenuButton
                className="data-[active=true]:font-normal"
                data-testid={`open-playground-${session.sid}`}
                isActive={overlaySid === session.sid}
                onClick={() => showPlaygroundSession(session.sid, { preferSidePanel: true })}
                tooltip={session.name}
                type="button"
              >
                <AppWindow className="h-4 w-4" />
                <SidebarMenuLabel>{session.name}</SidebarMenuLabel>
              </SidebarMenuButton>
              {session.hasUpdate ? (
                <SidebarMenuBadge
                  className="right-2 rounded-full bg-primary/15 px-1.5 text-2xs text-primary"
                  data-testid={`playground-update-chip-${session.sid}`}
                >
                  New
                </SidebarMenuBadge>
              ) : null}
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
