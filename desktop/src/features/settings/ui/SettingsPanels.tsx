import { useState, useMemo, useRef } from "react";
import {
  BellRing,
  Bot,
  Check,
  Cpu,
  Download,
  FlaskConical,
  Keyboard,
  LayoutTemplate,
  LockKeyhole,
  Monitor,
  MonitorCog,
  Moon,
  Plug,
  Search,
  Smile,
  Stethoscope,
  Sun,
  UserRound,
  type LucideIcon,
} from "lucide-react";
import type {
  DesktopNotificationPermissionState,
  NotificationSettings,
} from "@/features/notifications/hooks";
import type { SoundName, SoundSlot } from "@/features/notifications/lib/sound";
import { RelayMembersSettingsCard } from "@/features/relay-members/ui/RelayMembersSettingsCard";
import { CustomEmojiSettingsCard } from "@/features/custom-emoji/ui/CustomEmojiSettingsCard";
import { ConnectionsSettingsCard } from "@/features/spotify/ui/ConnectionsSettingsCard";
import { cn } from "@/shared/lib/cn";
import {
  ACCENT_COLORS,
  NEUTRAL_ACCENT,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import { SYNTAX_THEMES, isLightTheme } from "@/shared/theme/theme-loader";
import { Switch } from "@/shared/ui/switch";
import { ChannelTemplatesSettingsCard } from "./ChannelTemplatesSettingsCard";
import { DoctorSettingsPanel } from "./DoctorSettingsPanel";
import { ExperimentalFeaturesCard } from "./ExperimentalFeaturesCard";
import { KeyboardShortcutsCard } from "./KeyboardShortcutsCard";
import { MeshComputeSettingsCard } from "@/features/mesh-compute/ui/MeshComputeSettingsCard";
import { NotificationSettingsCard } from "./NotificationSettingsCard";
import { PreventSleepSettingsCard } from "./PreventSleepSettingsCard";
import { ProfileSettingsCard } from "./ProfileSettingsCard";
import { UpdateChecker } from "../UpdateChecker";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export type SettingsSection =
  | "profile"
  | "notifications"
  | "connections"
  | "experimental"
  | "agents"
  | "channel-templates"
  | "compute"
  | "appearance"
  | "shortcuts"
  | "relay-members"
  | "custom-emoji"
  | "updates"
  | "doctor";

export const DEFAULT_SETTINGS_SECTION: SettingsSection = "profile";

const SETTINGS_SECTION_VALUES: readonly SettingsSection[] = [
  "profile",
  "notifications",
  "connections",
  "experimental",
  "agents",
  "channel-templates",
  "compute",
  "appearance",
  "shortcuts",
  "relay-members",
  "custom-emoji",
  "updates",
  "doctor",
];

export function isSettingsSection(value: unknown): value is SettingsSection {
  return (
    typeof value === "string" &&
    (SETTINGS_SECTION_VALUES as readonly string[]).includes(value)
  );
}

export type SettingsSectionDescriptor = {
  value: SettingsSection;
  label: string;
  icon: LucideIcon;
  /** If set, this section is only visible when the feature is enabled */
  featureGate?: string;
};

export type SettingsPanelProps = {
  currentPubkey?: string;
  fallbackDisplayName?: string;
  isUpdatingDesktopNotifications: boolean;
  notificationErrorMessage: string | null;
  notificationPermission: DesktopNotificationPermissionState;
  notificationSettings: NotificationSettings;
  onSetDesktopNotificationsEnabled: (enabled: boolean) => Promise<boolean>;
  onSetHomeBadgeEnabled: (enabled: boolean) => void;
  onSetSlotAlertsEnabled: (slot: SoundSlot, enabled: boolean) => void;
  onSetNotifyWhileViewing: (enabled: boolean) => void;
  onSetAllSlotAlertsEnabled: (enabled: boolean) => void;
  onSetSoundForSlot: (slot: SoundSlot, name: SoundName) => void;
};

export const settingsSections: SettingsSectionDescriptor[] = [
  {
    value: "appearance",
    label: "Appearance",
    icon: MonitorCog,
  },
  {
    value: "profile",
    label: "Profile",
    icon: UserRound,
  },
  {
    value: "notifications",
    label: "Notifications",
    icon: BellRing,
  },
  {
    value: "connections",
    label: "Connections",
    icon: Plug,
  },
  {
    value: "experimental",
    label: "Experiments",
    icon: FlaskConical,
  },
  {
    value: "agents",
    label: "Agents",
    icon: Bot,
    featureGate: "managed-agents",
  },
  {
    value: "channel-templates",
    label: "Templates",
    icon: LayoutTemplate,
    featureGate: "channel-templates",
  },
  {
    value: "compute",
    label: "Compute",
    icon: Cpu,
  },
  {
    value: "shortcuts",
    label: "Shortcuts",
    icon: Keyboard,
  },
  {
    value: "relay-members",
    label: "Relay Access",
    icon: LockKeyhole,
  },
  {
    value: "custom-emoji",
    label: "Custom Emoji",
    icon: Smile,
    featureGate: "custom-emoji",
  },
  {
    value: "updates",
    label: "Updates",
    icon: Download,
  },
  {
    value: "doctor",
    label: "Doctor",
    icon: Stethoscope,
    featureGate: "doctor",
  },
];

function formatThemeLabel(name: string): string {
  return name
    .split("-")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

function ThemeSettingsCard() {
  const {
    setTheme,
    themeName,
    selectedThemeName,
    isDark,
    accentColor,
    setAccentColor,
    followSystem,
    setFollowSystem,
  } = useTheme();
  const [search, setSearch] = useState("");
  const didScrollRef = useRef(false);
  const activeRef = (node: HTMLButtonElement | null) => {
    if (node && !didScrollRef.current) {
      didScrollRef.current = true;
      node.scrollIntoView({ block: "center" });
    }
  };

  const selectedTheme = selectedThemeName;

  const filtered = useMemo(() => {
    const q = search.toLowerCase().trim();
    if (!q) return SYNTAX_THEMES;
    return SYNTAX_THEMES.filter((name) => name.includes(q));
  }, [search]);

  return (
    <section
      className="flex min-h-0 flex-1 flex-col"
      data-testid="settings-theme"
    >
      <SettingsSectionHeader
        title="Appearance"
        description="Choose a theme for Buzz."
      />

      <div className="relative mb-3 shrink-0">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <input
          autoCapitalize="none"
          autoCorrect="off"
          className="w-full rounded-lg border border-border/70 bg-background/70 py-2 pl-9 pr-3 text-sm placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search themes..."
          spellCheck={false}
          type="text"
          value={search}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-border/70 bg-background/70">
        {filtered.length === 0 ? (
          <p className="px-3 py-4 text-center text-sm text-muted-foreground">
            No themes match your search.
          </p>
        ) : (
          filtered.map((name) => {
            const isActive = selectedTheme === name;
            const isEffective = themeName === name;
            const light = isLightTheme(name);

            return (
              <button
                aria-pressed={isActive}
                className={cn(
                  "flex w-full items-center gap-3 px-3 py-2 text-left text-sm transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                  isActive
                    ? "bg-primary/10 text-foreground"
                    : isEffective && followSystem
                      ? "bg-primary/5 text-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                )}
                data-testid={`theme-option-${name}`}
                key={name}
                onClick={() => setTheme(name)}
                ref={isActive ? activeRef : undefined}
                type="button"
              >
                {light ? (
                  <Sun className="h-4 w-4 shrink-0" />
                ) : (
                  <Moon className="h-4 w-4 shrink-0" />
                )}
                <span className="flex-1 truncate">
                  {formatThemeLabel(name)}
                </span>
                {isActive && (
                  <Check className="h-4 w-4 shrink-0 text-primary" />
                )}
                {!isActive && isEffective && followSystem && (
                  <Monitor className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                )}
              </button>
            );
          })
        )}
      </div>

      <div className="mt-4 flex shrink-0 flex-col gap-3 pb-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <h3 className="mb-2 text-sm font-medium">Accent color</h3>
          <div className="flex flex-wrap gap-2">
            {ACCENT_COLORS.map((color) => {
              const isNeutral = color.value === NEUTRAL_ACCENT;
              const swatchColor = isNeutral
                ? "hsl(var(--foreground))"
                : color.value;
              const checkClassName =
                isNeutral && isDark ? "text-black" : "text-white";

              return (
                <button
                  className={cn(
                    "flex h-7 w-7 items-center justify-center rounded-full border border-border/50 transition-transform hover:scale-110",
                    accentColor === color.value &&
                      "ring-2 ring-ring ring-offset-2 ring-offset-background",
                  )}
                  data-testid={`accent-color-${color.name.toLowerCase()}`}
                  key={color.value}
                  onClick={() => setAccentColor(color.value)}
                  style={{ backgroundColor: swatchColor }}
                  title={color.name}
                  type="button"
                >
                  {accentColor === color.value && (
                    <Check className={cn("h-4 w-4", checkClassName)} />
                  )}
                </button>
              );
            })}
          </div>
        </div>

        <label
          className="flex cursor-pointer items-center gap-3 text-sm font-medium text-foreground"
          htmlFor="follow-system-switch"
        >
          <span className="min-w-0 truncate">Use system setting</span>
          <Switch
            checked={followSystem}
            data-testid="follow-system-toggle"
            id="follow-system-switch"
            onCheckedChange={setFollowSystem}
          />
        </label>
      </div>
    </section>
  );
}

export function renderSettingsSection(
  section: SettingsSection,
  props: SettingsPanelProps,
): React.ReactNode {
  switch (section) {
    case "profile":
      return (
        <ProfileSettingsCard
          currentPubkey={props.currentPubkey}
          fallbackDisplayName={props.fallbackDisplayName}
        />
      );
    case "notifications":
      return (
        <NotificationSettingsCard
          isUpdatingDesktopNotifications={props.isUpdatingDesktopNotifications}
          notificationErrorMessage={props.notificationErrorMessage}
          notificationPermission={props.notificationPermission}
          notificationSettings={props.notificationSettings}
          onSetDesktopNotificationsEnabled={
            props.onSetDesktopNotificationsEnabled
          }
          onSetHomeBadgeEnabled={props.onSetHomeBadgeEnabled}
          onSetSlotAlertsEnabled={props.onSetSlotAlertsEnabled}
          onSetNotifyWhileViewing={props.onSetNotifyWhileViewing}
          onSetAllSlotAlertsEnabled={props.onSetAllSlotAlertsEnabled}
          onSetSoundForSlot={props.onSetSoundForSlot}
        />
      );
    case "connections":
      return <ConnectionsSettingsCard />;
    case "experimental":
      return <ExperimentalFeaturesCard />;
    case "agents":
      return <PreventSleepSettingsCard />;
    case "channel-templates":
      return <ChannelTemplatesSettingsCard />;
    case "compute":
      return <MeshComputeSettingsCard />;
    case "appearance":
      return <ThemeSettingsCard />;
    case "shortcuts":
      return <KeyboardShortcutsCard />;
    case "relay-members":
      return <RelayMembersSettingsCard currentPubkey={props.currentPubkey} />;
    case "custom-emoji":
      return <CustomEmojiSettingsCard />;
    case "updates":
      return <UpdateChecker />;
    case "doctor":
      return <DoctorSettingsPanel />;
    default: {
      const exhaustiveCheck: never = section;
      return exhaustiveCheck;
    }
  }
}
