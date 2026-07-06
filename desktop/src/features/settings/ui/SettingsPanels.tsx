import { useMemo } from "react";
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
  MonitorCog,
  Smartphone,
  Smile,
  Stethoscope,
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
import { cn } from "@/shared/lib/cn";
import {
  ACCENT_COLORS,
  NEUTRAL_ACCENT,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import {
  LIGHT_THEMES,
  SYNTAX_THEMES,
  type SyntaxThemeName,
  getThemePair,
} from "@/shared/theme/theme-loader";
import {
  ThemePreviewFrame,
  type ThemePreviewVars,
} from "@/shared/theme/ThemePreviewFrame";
import {
  getThemeFallbackPreviewVars,
  useThemePreviewVars,
  withAccentPreviewVars,
} from "@/shared/theme/useThemePreviewVars";
import { ChannelTemplatesSettingsCard } from "./ChannelTemplatesSettingsCard";
import { DoctorSettingsPanel } from "./DoctorSettingsPanel";
import { ExperimentalFeaturesCard } from "./ExperimentalFeaturesCard";
import { KeyboardShortcutsCard } from "./KeyboardShortcutsCard";
import { MeshComputeSettingsCard } from "@/features/mesh-compute/ui/MeshComputeSettingsCard";
import { MobilePairingCard } from "./MobilePairingCard";
import { NotificationSettingsCard } from "./NotificationSettingsCard";
import { PreventSleepSettingsCard } from "./PreventSleepSettingsCard";
import { ProfileSettingsCard } from "./ProfileSettingsCard";
import { UpdateChecker } from "../UpdateChecker";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export type SettingsSection =
  | "profile"
  | "notifications"
  | "experimental"
  | "agents"
  | "channel-templates"
  | "compute"
  | "appearance"
  | "shortcuts"
  | "relay-members"
  | "custom-emoji"
  | "mobile"
  | "updates"
  | "doctor";

export const DEFAULT_SETTINGS_SECTION: SettingsSection = "profile";

const SETTINGS_SECTION_VALUES: readonly SettingsSection[] = [
  "profile",
  "notifications",
  "experimental",
  "agents",
  "channel-templates",
  "compute",
  "appearance",
  "shortcuts",
  "relay-members",
  "custom-emoji",
  "mobile",
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
    value: "mobile",
    label: "Mobile",
    icon: Smartphone,
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

/**
 * Categorize themes into three groups:
 * 1. Paired — themes with both a light and dark variant (auto-switches with system)
 * 2. Light-only — light themes with no dark counterpart
 * 3. Dark-only — dark themes with no light counterpart
 *
 * For paired themes, we deduplicate by only keeping the light member
 * (the dark member is shown alongside it as a preview).
 */
function useThemeCategories() {
  return useMemo(() => {
    const pairedLight: SyntaxThemeName[] = [];
    const lightOnly: SyntaxThemeName[] = [];
    const darkOnly: SyntaxThemeName[] = [];

    // Track which themes are the "dark side" of a pair so we skip them
    const darkPairMembers = new Set<string>();
    for (const name of SYNTAX_THEMES) {
      if (LIGHT_THEMES.has(name)) {
        const pair = getThemePair(name);
        if (pair) {
          darkPairMembers.add(pair);
        }
      }
    }

    for (const name of SYNTAX_THEMES) {
      // Skip dark members of pairs — they'll be shown alongside their light counterpart
      if (darkPairMembers.has(name)) continue;

      if (LIGHT_THEMES.has(name)) {
        const pair = getThemePair(name);
        if (pair) {
          pairedLight.push(name);
        } else {
          lightOnly.push(name);
        }
      } else {
        darkOnly.push(name);
      }
    }

    return { pairedLight, lightOnly, darkOnly };
  }, []);
}

function PairedThemeTile({
  isActive,
  lightName,
  lightVars,
  darkVars,
  onSelect,
}: {
  isActive: boolean;
  lightName: SyntaxThemeName;
  lightVars: ThemePreviewVars | null;
  darkVars: ThemePreviewVars | null;
  onSelect: () => void;
}) {
  return (
    <button
      aria-pressed={isActive}
      className={cn(
        "group flex min-w-0 flex-col rounded-lg border bg-background/70 p-2 text-left transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        isActive
          ? "border-primary text-foreground shadow-sm"
          : "border-border/70 text-muted-foreground hover:border-border hover:bg-accent/70 hover:text-accent-foreground",
      )}
      data-testid={`theme-pair-${lightName}`}
      onClick={onSelect}
      type="button"
    >
      <div className="flex gap-2">
        <ThemePreviewFrame className="h-20 w-[120px]" vars={lightVars} />
        <ThemePreviewFrame className="h-20 w-[120px]" vars={darkVars} />
      </div>

      <div className="mt-2 flex min-h-6 items-center gap-2 px-1">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {formatThemeLabel(
            lightName.replace(/-(?:light|latte|dawn|lotus|ochin)$/, ""),
          )}
        </span>
        {isActive ? <Check className="h-4 w-4 shrink-0 text-primary" /> : null}
      </div>
    </button>
  );
}

function SingleThemeTile({
  isActive,
  name,
  vars,
  onSelect,
}: {
  isActive: boolean;
  name: SyntaxThemeName;
  vars: ThemePreviewVars | null;
  onSelect: () => void;
}) {
  return (
    <button
      aria-pressed={isActive}
      className={cn(
        "group flex w-[174px] min-w-0 flex-col rounded-lg border bg-background/70 p-2 text-left transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        isActive
          ? "border-primary text-foreground shadow-sm"
          : "border-border/70 text-muted-foreground hover:border-border hover:bg-accent/70 hover:text-accent-foreground",
      )}
      data-testid={`theme-option-${name}`}
      onClick={onSelect}
      type="button"
    >
      <ThemePreviewFrame vars={vars} />

      <div className="mt-2 flex min-h-6 items-center gap-2 px-1">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {formatThemeLabel(name)}
        </span>
        {isActive ? <Check className="h-4 w-4 shrink-0 text-primary" /> : null}
      </div>
    </button>
  );
}

function ThemeSettingsCard() {
  const {
    setTheme,
    selectedThemeName,
    isDark,
    accentColor,
    setAccentColor,
    setFollowSystem,
  } = useTheme();

  const previewVarsByTheme = useThemePreviewVars();
  const { pairedLight, lightOnly, darkOnly } = useThemeCategories();

  const getVars = (name: SyntaxThemeName) =>
    withAccentPreviewVars(
      previewVarsByTheme[name] ?? getThemeFallbackPreviewVars(name),
      accentColor,
    );

  /** Check if a paired theme (by its light member) is the active selection */
  const isPairActive = (lightName: SyntaxThemeName) => {
    const darkName = getThemePair(lightName);
    return selectedThemeName === lightName || selectedThemeName === darkName;
  };

  const handleSelectPair = (lightName: SyntaxThemeName) => {
    // Selecting a paired theme auto-enables follow-system
    setTheme(lightName);
    setFollowSystem(true);
  };

  const handleSelectSingle = (name: SyntaxThemeName) => {
    setTheme(name);
    // Single themes don't follow system — they're always one mode
    setFollowSystem(false);
  };

  return (
    <section
      className="flex min-h-0 flex-1 flex-col overflow-y-auto"
      data-testid="settings-theme"
    >
      <SettingsSectionHeader
        title="Appearance"
        description="Choose a theme for Buzz."
      />

      {/* Section 1: Paired themes (follow system) */}
      <div className="mb-6">
        <h3 className="mb-1 text-sm font-medium text-foreground">
          Adapts to system
        </h3>
        <p className="mb-3 text-2xs text-muted-foreground">
          Automatically switches between light and dark with your system
          preferences.
        </p>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3">
          {pairedLight.map((lightName) => {
            const darkName = getThemePair(lightName);
            if (!darkName) return null;
            return (
              <PairedThemeTile
                darkVars={getVars(darkName)}
                isActive={isPairActive(lightName)}
                key={lightName}
                lightName={lightName}
                lightVars={getVars(lightName)}
                onSelect={() => handleSelectPair(lightName)}
              />
            );
          })}
        </div>
      </div>

      {/* Section 2: Light-only themes */}
      {lightOnly.length > 0 && (
        <div className="mb-6">
          <h3 className="mb-1 text-sm font-medium text-foreground">Light</h3>
          <p className="mb-3 text-2xs text-muted-foreground">
            Always uses a light appearance.
          </p>
          <div className="grid grid-cols-[repeat(auto-fill,174px)] gap-3">
            {lightOnly.map((name) => (
              <SingleThemeTile
                isActive={selectedThemeName === name}
                key={name}
                name={name}
                onSelect={() => handleSelectSingle(name)}
                vars={getVars(name)}
              />
            ))}
          </div>
        </div>
      )}

      {/* Section 3: Dark-only themes */}
      {darkOnly.length > 0 && (
        <div className="mb-6">
          <h3 className="mb-1 text-sm font-medium text-foreground">Dark</h3>
          <p className="mb-3 text-2xs text-muted-foreground">
            Always uses a dark appearance.
          </p>
          <div className="grid grid-cols-[repeat(auto-fill,174px)] gap-3">
            {darkOnly.map((name) => (
              <SingleThemeTile
                isActive={selectedThemeName === name}
                key={name}
                name={name}
                onSelect={() => handleSelectSingle(name)}
                vars={getVars(name)}
              />
            ))}
          </div>
        </div>
      )}

      {/* Accent color picker */}
      <div className="mt-2 shrink-0 pb-2">
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
    case "mobile":
      return <MobilePairingCard currentPubkey={props.currentPubkey} />;
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
