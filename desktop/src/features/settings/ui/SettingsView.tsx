import * as React from "react";
import { getVersion } from "@tauri-apps/api/app";
import { ArrowLeft } from "lucide-react";

import { useMyRelayMembershipQuery } from "@/features/community-members/hooks";
import { getFeature } from "@/shared/features/manifest";
import {
  resolveEnabled,
  useFeatureSnapshot,
} from "@/shared/features/useFeatureEnabled";
import { cn } from "@/shared/lib/cn";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/shared/ui/sidebar";
import {
  renderSettingsSection,
  settingsSections,
  type SettingsPanelProps,
  type SettingsSection,
  type SettingsSectionDescriptor,
} from "./SettingsPanels";

export {
  DEFAULT_SETTINGS_SECTION,
  type SettingsSection,
} from "./SettingsPanels";

type SettingsViewProps = SettingsPanelProps & {
  onClose: () => void;
  onSectionChange: (section: SettingsSection) => void;
  section: SettingsSection;
};

const settingsNavGroups: Array<{
  label: string;
  sections: SettingsSection[];
}> = [
  {
    label: "Personal",
    sections: [
      "profile",
      "appearance",
      "notifications",
      "shortcuts",
      "custom-emoji",
      "local-archive",
    ],
  },
  {
    label: "Communities",
    sections: ["channel-templates", "community-members"],
  },
  {
    label: "App",
    sections: [
      "agents",
      "compute",
      "experimental",
      "mobile",
      "updates",
      "doctor",
    ],
  },
];

function SettingsSectionButton({
  active,
  onSelect,
  section,
}: {
  active: boolean;
  onSelect: (section: SettingsSection) => void;
  section: (typeof settingsSections)[number];
}) {
  const Icon = section.icon;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        aria-pressed={active}
        data-testid={`settings-nav-${section.value}`}
        isActive={active}
        onClick={() => onSelect(section.value)}
        tooltip={section.label}
        type="button"
      >
        <Icon
          className={cn(
            "h-4 w-4 shrink-0 transition-colors",
            active
              ? "text-sidebar-active-foreground"
              : "text-sidebar-foreground/70",
          )}
        />
        <span>{section.label}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

export function SettingsView({
  currentPubkey,
  fallbackDisplayName,
  isUpdatingDesktopNotifications,
  notificationErrorMessage,
  notificationPermission,
  notificationSettings,
  onClose,
  onSectionChange,
  onSetDesktopNotificationsEnabled,
  onSetHomeBadgeEnabled,
  onSetSlotAlertsEnabled,
  onSetNotifyWhileViewing,
  onSetAllSlotAlertsEnabled,
  onSetSoundForSlot,
  section,
}: SettingsViewProps) {
  const { isMobile, open: sidebarOpen, setOpen: setSidebarOpen } = useSidebar();
  const myMembershipQuery = useMyRelayMembershipQuery();
  const featureState = useFeatureSnapshot();
  const visibleSections = React.useMemo(() => {
    const membership = myMembershipQuery.data;

    return settingsSections.filter((s) => {
      // Feature gate check. Manifest is preview-only — if the gate id is in
      // the manifest, it's preview and needs an opt-in; if it's not, it's
      // stable and renders unconditionally (fail-open).
      if (s.featureGate) {
        const feature = getFeature(s.featureGate);
        if (feature && !resolveEnabled(s.featureGate, featureState)) {
          return false;
        }
      }
      // Community members requires admin/owner role
      if (s.value === "community-members") {
        return (
          membership != null &&
          (membership.role === "owner" || membership.role === "admin")
        );
      }
      return true;
    });
  }, [myMembershipQuery.data, featureState]);

  const [isLoaded, setIsLoaded] = React.useState(false);
  const [appVersion, setAppVersion] = React.useState<string | null>(null);

  React.useEffect(() => {
    const frameId = window.requestAnimationFrame(() => setIsLoaded(true));
    return () => window.cancelAnimationFrame(frameId);
  }, []);

  React.useEffect(() => {
    void getVersion().then(setAppVersion);
  }, []);

  React.useEffect(() => {
    if (!visibleSections.some((entry) => entry.value === section)) {
      onSectionChange(visibleSections[0]?.value ?? "appearance");
    }
  }, [onSectionChange, section, visibleSections]);

  React.useEffect(() => {
    if (!isMobile && !sidebarOpen) {
      setSidebarOpen(true);
    }
  }, [isMobile, setSidebarOpen, sidebarOpen]);

  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const visibleSectionByValue = React.useMemo(
    () => new Map(visibleSections.map((entry) => [entry.value, entry])),
    [visibleSections],
  );
  const visibleNavGroups = React.useMemo(
    () =>
      settingsNavGroups
        .map((group) => ({
          ...group,
          sections: group.sections
            .map((value) => visibleSectionByValue.get(value))
            .filter(
              (entry): entry is SettingsSectionDescriptor => entry != null,
            ),
        }))
        .filter((group) => group.sections.length > 0),
    [visibleSectionByValue],
  );

  return (
    <>
      <Sidebar
        className="!border-r-0"
        collapsible="offcanvas"
        data-testid="settings-sidebar"
        variant="sidebar"
      >
        <SidebarHeader
          className="cursor-default select-none pt-11"
          data-tauri-drag-region
        >
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid="settings-back-to-app"
                onClick={onClose}
                tooltip="Back to app"
                type="button"
              >
                <ArrowLeft className="h-4 w-4" />
                <span>Back to app</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarHeader>

        <SidebarContent>
          {visibleNavGroups.map((group) => (
            <SidebarGroup key={group.label}>
              <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
              <SidebarGroupContent>
                <SidebarMenu aria-label={`${group.label} settings sections`}>
                  {group.sections.map((entry) => (
                    <SettingsSectionButton
                      active={entry.value === section}
                      key={entry.value}
                      onSelect={onSectionChange}
                      section={entry}
                    />
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          ))}
        </SidebarContent>

        <SidebarFooter>
          {appVersion ? (
            <p className="px-2 pb-1 text-xs text-sidebar-foreground/45">
              v{appVersion}
            </p>
          ) : null}
        </SidebarFooter>
      </Sidebar>

      <SidebarInset
        className={cn(
          "isolate relative min-h-0 min-w-0 overflow-hidden bg-sidebar motion-safe:transition-opacity motion-safe:duration-200",
          isLoaded ? "opacity-100" : "opacity-0",
        )}
        data-testid="settings-view"
      >
        <div
          aria-hidden="true"
          className="absolute inset-x-0 top-0 z-10 h-11"
          data-tauri-drag-region
        />
        <div className="relative z-10 ml-px mt-px flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-background pt-11 shadow-[-1px_-1px_0_0_hsl(var(--sidebar-border)/0.45)]">
          <section className="min-h-0 flex-1 overflow-y-auto px-5 pb-12 pt-4 sm:px-6">
            <div
              className="mx-auto flex h-full w-full max-w-4xl flex-col gap-4"
              data-testid={`settings-panel-${section}`}
            >
              {renderSettingsSection(section, {
                currentPubkey,
                fallbackDisplayName,
                isUpdatingDesktopNotifications,
                notificationErrorMessage,
                notificationPermission,
                notificationSettings,
                onSetDesktopNotificationsEnabled,
                onSetHomeBadgeEnabled,
                onSetSlotAlertsEnabled,
                onSetNotifyWhileViewing,
                onSetAllSlotAlertsEnabled,
                onSetSoundForSlot,
              })}
            </div>
          </section>
        </div>
      </SidebarInset>
    </>
  );
}
