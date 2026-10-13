import { isTauri } from "@tauri-apps/api/core";
import { useRouter, useRouterState } from "@tanstack/react-router";
import * as React from "react";
import { getVersion } from "@tauri-apps/api/app";
import { AlertCircle, ArrowLeft, ChevronRight, LoaderCircle, RefreshCw } from "lucide-react";
import { usePhoneLayout } from "@/shared/lib/phoneLayout";

import { isAdminOnlySettingsSection } from "@/features/community-bots/lib/settingsVisibility";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import {
  canManageCommunityMembers,
  shouldWarnMissingMembershipSnapshot,
} from "@/shared/api/relayMembers";
import {
  getFeature,
  resolveEnabled,
  useFeatureSnapshot,
} from "@/shared/features";
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
  DESKTOP_ONLY_SETTINGS_SECTIONS,
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

export const settingsNavGroups: Array<{
  label: string;
  sections: SettingsSection[];
}> = [
  {
    label: "Personal",
    sections: [
      "profile",
      "appearance",
      "notifications",
      "voice",
      "huddle",
      "shortcuts",
      "buzz-term",
      ...(isTauri() ? (["pinned-sites"] as SettingsSection[]) : []), // web: pinned sites are desktop-only (docs/pwa-web-differences.md #1)
      "custom-emoji",
      "local-archive",
      "channel-templates",
    ],
  },
  {
    label: "Communities",
    sections: [
      "hosted-communities",
      "community-members",
      "community-sections",
      "bots",
      "relay-admin",
    ],
  },
  {
    label: "App",
    sections: ["agents", "compute", "experimental", "mobile", "updates"],
  },
];

/**
 * Keep the Settings chrome (sidebar + shell) mounted if a section card throws.
 * Without this, RootErrorBoundary replaces the whole window — including the
 * settings nav — with a degraded splash.
 */
class SettingsSectionErrorBoundary extends React.Component<
  { children: React.ReactNode; section: SettingsSection },
  { error: Error | null }
> {
  override state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error): { error: Error } {
    return { error };
  }

  override componentDidCatch(error: Error, info: React.ErrorInfo): void {
    console.error(
      `[Settings] "${this.props.section}" failed to render:`,
      error,
      info,
    );
  }

  override componentDidUpdate(prevProps: {
    children: React.ReactNode;
    section: SettingsSection;
  }): void {
    if (prevProps.section !== this.props.section && this.state.error) {
      this.setState({ error: null });
    }
  }

  override render(): React.ReactNode {
    if (this.state.error) {
      return (
        <p className="px-4 py-8 text-sm text-muted-foreground">
          This settings page failed to load. Choose another section or reopen
          Settings.
        </p>
      );
    }
    return this.props.children;
  }
}

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
    return settingsSections.filter((s) => {
      // Web: desktop-only sections (Compute, Mobile, Updates, Hosted communities, Invites, Admin).
      if (DESKTOP_ONLY_SETTINGS_SECTIONS.includes(s.value) && !isTauri()) return false;
      // Feature gate check. Manifest is preview-only — if the gate id is in
      // the manifest, it's preview and needs an opt-in; if it's not, it's
      // stable and renders unconditionally (fail-open).
      if (s.featureGate) {
        const feature = getFeature(s.featureGate);
        if (
          feature &&
          !resolveEnabled(s.featureGate, featureState, feature.defaultEnabled)
        ) {
          return false;
        }
      }
      // Invites and member management require a discovered owner/admin role.
      // Open relays have no membership snapshot or invite controls.
      if (isAdminOnlySettingsSection(s.value)) {
        return canManageCommunityMembers(myMembershipQuery.data);
      }
      // Relay admin surfaces the relay admin console. Always reachable so an
      // operator can enter a manual origin even when NIP-11 discovery is
      // absent, invalid, or pending — hiding the entry would lock them out of
      // the only place to configure one. Auth still gates the panel itself.
      if (s.value === "relay-admin") {
        return true;
      }
      return true;
    });
  }, [myMembershipQuery.data, featureState]);

  const isPhone = usePhoneLayout();
  // Phone: Settings opens on the section list; a section opens full screen as its own history entry, so the shared
  // nav row's Back returns to the list, and Back from the list returns to wherever Settings was opened from (e.g. Me).
  const router = useRouter();
  // Keyed to the href it was pushed on (router navigations copy state forward; see phoneTabs.ts).
  const phoneDetail = useRouterState({
    select: (state) =>
      (state.location.state as { phoneSettingsDetail?: unknown } | undefined)?.phoneSettingsDetail ===
      router.history.location.href,
  });
  const phoneShowList = !phoneDetail;
  const openPhoneSection = React.useCallback(
    (value: SettingsSection) => {
      const href = `/settings?section=${encodeURIComponent(value)}`;
      router.history.push(href, { phoneSettingsDetail: href } as never);
    },
    [router],
  );
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

  if (isPhone) {
    const activeEntry = visibleSectionByValue.get(section);
    return (
      <div className="phone-settings" data-testid="phone-settings">
        <div className="phone-settings-header">
          <h1 className="truncate text-base font-semibold">
            {phoneShowList ? "Settings" : (activeEntry?.label ?? "Settings")}
          </h1>
        </div>
        {phoneShowList ? (
          <nav aria-label="Settings sections" className="phone-settings-scroll" data-testid="phone-settings-list">
            {visibleNavGroups.map((group) => (
              <div className="phone-settings-group" key={group.label}>
                <p className="px-4 pb-1 pt-4 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  {group.label}
                </p>
                <ul className="divide-y divide-border border-y border-border bg-background">
                  {group.sections.map((entry) => {
                    const Icon = entry.icon;
                    return (
                      <li key={entry.value}>
                        <button
                          className="flex w-full items-center gap-3 px-4 py-3 text-left text-sm active:bg-muted"
                          data-testid={`settings-nav-${entry.value}`}
                          onClick={() => openPhoneSection(entry.value)}
                          type="button"
                        >
                          <Icon className="h-5 w-5 text-muted-foreground" />
                          <span className="flex-1">{entry.label}</span>
                          <ChevronRight className="h-4 w-4 text-muted-foreground" />
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
            {appVersion ? (
              <p className="px-4 py-4 text-xs text-muted-foreground" data-testid="settings-version">
                v{appVersion}
              </p>
            ) : null}
          </nav>
        ) : (
          <section className="phone-settings-scroll px-4 pb-8 pt-3" data-testid="settings-content-scroll">
            <div className="flex w-full flex-col gap-4" data-testid={`settings-panel-${section}`}>
              <SettingsSectionErrorBoundary section={section}>
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
              </SettingsSectionErrorBoundary>
            </div>
          </section>
        )}
      </div>
    );
  }

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
          className={cn(
            "shrink-0 cursor-default select-none",
            topChromeBackdrop.height,
          )}
          data-tauri-drag-region
          data-testid="settings-sidebar-top-chrome"
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
          {/* Web: invites are desktop-only, so no invite-permission status here. */}
          {isTauri() && myMembershipQuery.isPending ? (
            <div
              className="mx-3 flex items-center gap-2 rounded-md border border-sidebar-border px-3 py-2 text-xs text-sidebar-foreground/70"
              data-testid="community-access-loading"
            >
              <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
              Checking invite permissions…
            </div>
          ) : null}
          {isTauri() && myMembershipQuery.isError ? (
            <div
              className="mx-3 space-y-2 rounded-md border border-destructive/40 px-3 py-2 text-xs text-sidebar-foreground"
              data-testid="community-access-error"
            >
              <div className="flex items-center gap-2">
                <AlertCircle className="h-3.5 w-3.5 text-destructive" />
                Invite settings could not be checked.
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
          {isTauri() && shouldWarnMissingMembershipSnapshot(myMembershipQuery.data) ? (
            <div
              className="mx-3 flex items-start gap-2 rounded-md border border-amber-500/40 px-3 py-2 text-xs text-sidebar-foreground"
              data-testid="community-access-snapshot-missing"
            >
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-500" />
              Invite settings are unavailable. Relay recovery may still be in
              progress.
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
          className={cn(
            "relative z-10 shrink-0 cursor-default select-none",
            topChromeBackdrop.height,
          )}
          data-tauri-drag-region
          data-testid="settings-top-chrome"
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
              className="mx-auto flex min-h-full w-full max-w-4xl flex-col gap-4"
              data-testid={`settings-panel-${section}`}
            >
              <SettingsSectionErrorBoundary section={section}>
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
              </SettingsSectionErrorBoundary>
            </div>
          </section>
        </div>
      </SidebarInset>
    </>
  );
}
