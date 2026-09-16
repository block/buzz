import { AppWindow, Columns2, MessageSquare, X } from "lucide-react";
import { useSyncExternalStore } from "react";

import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";

import {
  closeEmbeddedWindow,
  getEmbeddedWindowsStore,
  showEmbeddedWindow,
  subscribeEmbeddedWindows,
} from "../lib/embeddedWindows";
import {
  isEmbedInMainEnabled,
  subscribePopoutSettings,
  getPopoutSettings,
} from "../lib/popoutSettings";
import {
  focusPopoutWindow,
  getPopoutWindows,
  popoutKindFromLabel,
  subscribePopoutWindows,
} from "../lib/popoutWindows";

function PopoutRowIcon({ kind }: { kind: string | null }) {
  const className = "h-4 w-4";
  if (kind === "split") return <Columns2 className={className} />;
  if (kind === "thread") return <MessageSquare className={className} />;
  return <AppWindow className={className} />;
}

export function activateWindowsSectionRow(
  host: "embed" | "os",
  label: string,
): void {
  if (host === "embed") {
    showEmbeddedWindow(label);
    return;
  }
  void focusPopoutWindow(label);
}

export function WindowsSection() {
  const settings = useSyncExternalStore(
    subscribePopoutSettings,
    getPopoutSettings,
    getPopoutSettings,
  );
  const osRows = useSyncExternalStore(
    subscribePopoutWindows,
    getPopoutWindows,
    getPopoutWindows,
  );
  const embedded = useSyncExternalStore(
    subscribeEmbeddedWindows,
    getEmbeddedWindowsStore,
    getEmbeddedWindowsStore,
  );

  if (!settings.showWindowsSection) return null;

  const embedOn = isEmbedInMainEnabled();
  const rows = embedOn
    ? embedded.windows.map((row) => ({
        host: "embed" as const,
        label: row.label,
        title: row.title,
        kind: row.payload.kind,
        active: embedded.activeLabel === row.label,
      }))
    : osRows.map((row) => ({
        host: "os" as const,
        label: row.label,
        title: row.title,
        kind: popoutKindFromLabel(row.label),
        active: false,
      }));
  if (rows.length === 0) return null;

  return (
    <SidebarGroup data-testid="windows-section">
      <SidebarGroupLabel data-testid="windows-section-label">
        Windows
      </SidebarGroupLabel>
      <SidebarGroupContent>
        <SidebarMenu>
          {rows.map((row) => (
            <SidebarMenuItem key={row.label}>
              <SidebarMenuButton
                className="data-[active=true]:font-normal"
                data-testid={`open-window-${row.label}`}
                isActive={row.active}
                onClick={() => {
                  activateWindowsSectionRow(row.host, row.label);
                }}
                tooltip={row.title}
                type="button"
              >
                <PopoutRowIcon kind={row.kind} />
                <SidebarMenuLabel>{row.title}</SidebarMenuLabel>
              </SidebarMenuButton>
              {row.host === "embed" ? (
                <SidebarMenuAction
                  aria-label={`Close ${row.title}`}
                  data-testid={`close-window-${row.label}`}
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    closeEmbeddedWindow(row.label);
                  }}
                  showOnHover
                  type="button"
                >
                  <X />
                </SidebarMenuAction>
              ) : null}
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}
