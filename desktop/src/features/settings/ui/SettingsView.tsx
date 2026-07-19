import * as React from "react";
import { getVersion } from "@tauri-apps/api/app";
import { AlertCircle, ArrowLeft, LoaderCircle, RefreshCw } from "lucide-react";

import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { shouldWarnMissingMembershipSnapshot } from "@/shared/api/relayMembers";
import { getFeature } from "@/shared/features/manifest";
import {
  resolveEnabled,
  useFeatureSnapshot,
} from "@/shared/features/useFeatureEnabled";
import { topChromeBackdrop } from "@/shared/layout/chromeLayout";
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
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";
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
    sections: ["hosted-communities", "channel-templates", "community-members"],
  },
  {
    label: "App",
    sections: ["agents", "compute", "experimental", "mobile", "updates"],
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
        <SidebarMenuLabel>{section.label}</SidebarMenuLabel>
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
  const myMembershipQuery = useMyRelayMembershipLookupQuery();
  const featureState = useFeatureSnapshot();
  const visibleSections = React.useMemo(() => {
    const membership = myMembershipQuery.data?.membership;

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
        <div
          aria-hidden="true"
          className={cn("shrink-0", topChromeBackdrop.height)}
          data-tauri-drag-region
        />
        <SidebarHeader
          className="cursor-default select-none pb-0 pt-3"
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
          {myMembershipQuery.isPending ? (
            <div
              className="mx-3 flex items-center gap-2 rounded-md border border-sidebar-border px-3 py-2 text-xs text-sidebar-foreground/70"
              data-testid="community-access-loading"
            >
              <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
              Checking community access…
            </div>
          ) : null}
          {myMembershipQuery.isError ? (
            <div
              className="mx-3 space-y-2 rounded-md border border-destructive/40 px-3 py-2 text-xs text-sidebar-foreground"
              data-testid="community-access-error"
            >
              <div className="flex items-center gap-2">
                <AlertCircle className="h-3.5 w-3.5 text-destructive" />
                Community access could not be checked.
              </div>
              <button
                className="flex items-center gap-1.5 font-medium text-sidebar-foreground underline-offset-2 hover:underline"
                onClick={() => void myMembershipQuery.refetch()}
                type="button"
              >
                <RefreshCw className="h-3.5 w-3.5" />
                Try again
              </button>
            </div>
          ) : null}
          {shouldWarnMissingMembershipSnapshot(myMembershipQuery.data) ? (
            <div
              className="mx-3 flex items-start gap-2 rounded-md border border-amber-500/40 px-3 py-2 text-xs text-sidebar-foreground"
              data-testid="community-access-snapshot-missing"
            >
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" />
              Community access data is unavailable. Relay recovery may still be
              in progress.
            </div>
          ) : null}
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
            <p
              className="px-2 pb-1 text-xs text-sidebar-foreground/45"
              data-buzz-sidebar-secondary
              data-testid="settings-version"
            >
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
        data-buzz-shadow-viewport
        data-testid="settings-view"
      >
        <div
          aria-hidden="true"
          className={cn("relative z-10 shrink-0", topChromeBackdrop.height)}
          data-tauri-drag-region
        />
        <div
          className="relative z-10 mb-2 ml-px mr-2 mt-px flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background shadow-content-edge"
          data-buzz-content-surface
          data-testid="settings-content-surface"
        >
          <section
            className="min-h-0 flex-1 overflow-y-auto px-5 pb-12 pt-6 sm:px-6"
            data-testid="settings-content-scroll"
          >
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
