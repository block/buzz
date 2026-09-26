import {
  SIDEBAR_MENU_COUNT_IDS,
  SIDEBAR_MENU_COUNT_LABELS,
  setSidebarMenuCountPreference,
  useSidebarMenuCountPreferences,
} from "@/features/sidebar/lib/sidebarMenuCountsPreference";
import type { SidebarMenuCountId } from "@/features/sidebar/lib/sidebarMenuCounts";
import { Switch } from "@/shared/ui/switch";

import { SettingsOptionRow } from "./SettingsOptionGroup";

const COUNT_COPY: Record<SidebarMenuCountId, string> = {
  inbox: "Unread conversations in the sidebar Inbox row.",
  browsers: "Open browser groups in the sidebar Browsers row.",
  agents: "Running/total managed agents in the sidebar Agents row.",
  bots: "Visible community bots in the sidebar Bots row.",
};

/** Appearance toggles for numeric counts on each primary left-nav item. */
export function SidebarMenuCountsSetting() {
  const preferences = useSidebarMenuCountPreferences();

  return (
    <>
      <SettingsOptionRow data-testid="sidebar-menu-counts-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">Show counts on menu items</p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            Turn each sidebar count on or off independently.
          </p>
        </div>
      </SettingsOptionRow>
      {SIDEBAR_MENU_COUNT_IDS.map((id) => {
        const switchId = `sidebar-menu-counts-${id}-switch`;
        return (
          <SettingsOptionRow
            data-testid={`sidebar-menu-counts-${id}-row`}
            key={id}
          >
            <div className="min-w-0">
              <label className="text-sm font-medium" htmlFor={switchId}>
                {SIDEBAR_MENU_COUNT_LABELS[id]}
              </label>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {COUNT_COPY[id]}
              </p>
            </div>
            <Switch
              checked={preferences[id]}
              data-testid={`sidebar-menu-counts-${id}-toggle`}
              id={switchId}
              onCheckedChange={(checked) => {
                setSidebarMenuCountPreference(id, checked);
              }}
            />
          </SettingsOptionRow>
        );
      })}
    </>
  );
}
