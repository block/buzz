import { useMemo, useState } from "react";
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
  Moon,
  Smartphone,
  Smile,
  Stethoscope,
  Sun,
  SunMoon,
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
  SystemPreferencePreviewFrame,
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
      className="group flex w-[168px] shrink-0 flex-col items-center text-center focus-visible:outline-hidden"
      data-testid={`theme-pair-${lightName}`}
      onClick={onSelect}
      type="button"
    >
      <SystemPreferencePreviewFrame
        className={cn(
          "h-[112px] w-[168px] transition-shadow",
          isActive
            ? "ring-2 ring-primary ring-offset-2 ring-offset-background"
            : "group-hover:ring-2 group-hover:ring-border",
        )}
        darkVars={darkVars}
        lightVars={lightVars}
      />
      <span
        className={cn(
          "mt-1.5 w-full truncate text-2xs",
          isActive ? "font-medium text-foreground" : "text-muted-foreground",
        )}
      >
        {formatThemeLabel(
          lightName.replace(/-(?:light|latte|dawn|lotus|ochin)$/, ""),
        )}
      </span>
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
      className="group flex w-[168px] shrink-0 flex-col items-center text-center focus-visible:outline-hidden"
      data-testid={`theme-option-${name}`}
      onClick={onSelect}
      type="button"
    >
      <ThemePreviewFrame
        className={cn(
          "h-[112px] w-[168px] transition-shadow",
          isActive
            ? "ring-2 ring-primary ring-offset-2 ring-offset-background"
            : "group-hover:ring-2 group-hover:ring-border",
        )}
        vars={vars}
      />
      <span
        className={cn(
          "mt-1.5 w-full truncate text-2xs",
          isActive ? "font-medium text-foreground" : "text-muted-foreground",
        )}
      >
        {formatThemeLabel(name)}
      </span>
    </button>
  );
}

type AppearanceMode = "system" | "light" | "dark";

function ThemeSettingsCard() {
  const {
    setTheme,
    selectedThemeName,
    isDark,
    accentColor,
    setAccentColor,
    followSystem,
    setFollowSystem,
  } = useTheme();

  const previewVarsByTheme = useThemePreviewVars();
  const { pairedLight, lightOnly, darkOnly } = useThemeCategories();

  // Determine the active mode from current state
  const activeMode: AppearanceMode = followSystem
    ? "system"
    : isDark
      ? "dark"
      : "light";

  const [selectedMode, setSelectedMode] = useState<AppearanceMode>(activeMode);

  const getVars = (name: SyntaxThemeName) =>
    withAccentPreviewVars(
      previewVarsByTheme[name] ?? getThemeFallbackPreviewVars(name),
      accentColor,
    );

  // All light themes (paired light + light-only)
  const allLightThemes = useMemo(
    () => [...pairedLight, ...lightOnly],
    [pairedLight, lightOnly],
  );

  // All dark themes (paired dark + dark-only)
  const allDarkThemes = useMemo(() => {
    const pairedDark = pairedLight
      .map((l) => getThemePair(l))
      .filter(Boolean) as SyntaxThemeName[];
    return [...pairedDark, ...darkOnly];
  }, [pairedLight, darkOnly]);

  const handleModeSelect = (mode: AppearanceMode) => {
    setSelectedMode(mode);
  };

  const handleSelectTheme = (name: SyntaxThemeName) => {
    setTheme(name);
    if (selectedMode === "system") {
      setFollowSystem(true);
    } else {
      setFollowSystem(false);
    }
  };

  /** Check if a paired theme (by its light member) is the active selection */
  const isPairActive = (lightName: SyntaxThemeName) => {
    const darkName = getThemePair(lightName);
    return selectedThemeName === lightName || selectedThemeName === darkName;
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

      {/* Mode selector: System / Light / Dark */}
      <div className="mb-8 flex gap-2">
        {(
          [
            { mode: "system" as const, label: "System", Icon: SunMoon },
            { mode: "light" as const, label: "Light", Icon: Sun },
            { mode: "dark" as const, label: "Dark", Icon: Moon },
          ] as const
        ).map(({ mode, label, Icon }) => (
          <button
            aria-pressed={selectedMode === mode}
            className={cn(
              "flex items-center gap-2 rounded-lg border px-4 py-2 text-sm font-medium transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
              selectedMode === mode
                ? "border-primary bg-primary/10 text-foreground"
                : "border-border/70 text-muted-foreground hover:border-border hover:text-foreground",
            )}
            data-testid={`appearance-mode-${mode}`}
            key={mode}
            onClick={() => handleModeSelect(mode)}
            type="button"
          >
            <Icon className="h-4 w-4" />
            {label}
          </button>
        ))}
      </div>

      {/* Theme grid based on selected mode */}
      <div className="mb-6">
        <div className="flex flex-wrap gap-3 p-1">
          {selectedMode === "system" &&
            pairedLight.map((lightName) => {
              const darkName = getThemePair(lightName);
              if (!darkName) return null;
              return (
                <PairedThemeTile
                  darkVars={getVars(darkName)}
                  isActive={isPairActive(lightName)}
                  key={lightName}
                  lightName={lightName}
                  lightVars={getVars(lightName)}
                  onSelect={() => handleSelectTheme(lightName)}
                />
              );
            })}
          {selectedMode === "light" &&
            allLightThemes.map((name) => (
              <SingleThemeTile
                isActive={selectedThemeName === name}
                key={name}
                name={name}
                onSelect={() => handleSelectTheme(name)}
                vars={getVars(name)}
              />
            ))}
          {selectedMode === "dark" &&
            allDarkThemes.map((name) => (
              <SingleThemeTile
                isActive={selectedThemeName === name}
                key={name}
                name={name}
                onSelect={() => handleSelectTheme(name)}
                vars={getVars(name)}
              />
            ))}
        </div>
      </div>

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
