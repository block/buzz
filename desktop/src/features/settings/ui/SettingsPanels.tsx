import { useMemo, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  Archive,
  BellRing,
  Bot,
  ChevronDown,
  Cpu,
  Download,
  FlaskConical,
  Keyboard,
  LayoutTemplate,
  MessagesSquare,
  MonitorCog,
  Moon,
  ShieldAlert,
  Smartphone,
  Smile,
  Sun,
  SunMoon,
  Ticket,
  UserRound,
  Volume2,
  type LucideIcon,
} from "lucide-react";
import type {
  DesktopNotificationPermissionState,
  NotificationSettings,
} from "@/features/notifications/hooks";
import type { SoundName, SoundSlot } from "@/features/notifications/lib/sound";
import { CommunityMembersSettingsCard } from "@/features/community-members/ui/CommunityMembersSettingsCard";
import { CustomEmojiSettingsCard } from "@/features/custom-emoji/ui/CustomEmojiSettingsCard";
import { LocalArchiveSettingsCard } from "@/features/local-archive/ui/LocalArchiveSettingsCard";
import { cn } from "@/shared/lib/cn";
import { useCommunities } from "@/features/communities/useCommunities";
import { Badge } from "@/shared/ui/badge";
import { isBuzzTheme, useTheme } from "@/shared/theme/ThemeProvider";
import {
  LIGHT_THEMES,
  SYNTAX_THEMES,
  type SyntaxThemeName,
  getThemePair,
} from "@/shared/theme/theme-loader";
import {
  BUZZ_GRADIENT_STOPS,
  SystemPreferencePreviewFrame,
  ThemePreviewFrame,
  type ThemePreviewVars,
} from "@/shared/theme/ThemePreviewFrame";
import {
  getThemeFallbackPreviewVars,
  useThemePreviewVars,
  withAccentPreviewVars,
} from "@/shared/theme/useThemePreviewVars";
import { appearanceCommunityLabel } from "../lib/appearanceScopeCopy";
import {
  AccentPickerContent,
  ConversationDisplaySettings,
  GlassBackgroundSetting,
  LinkPreviewStyleSetting,
  ProminentActiveTabSetting,
  ThreadLayoutSetting,
} from "./AppearanceSettingsControls";
import { ChannelTemplatesSettingsCard } from "./ChannelTemplatesSettingsCard";
import { ExperimentalFeaturesCard } from "./ExperimentalFeaturesCard";
import { KeyboardShortcutsCard } from "./KeyboardShortcutsCard";
import { MeshComputeSettingsCard } from "@/features/mesh-compute/ui/MeshComputeSettingsCard";
import { MobilePairingCard } from "./MobilePairingCard";
import { ModerationQueueCard } from "./ModerationQueueCard";
import { NotificationSettingsCard } from "./NotificationSettingsCard";
import { AgentsSettingsPanel } from "./AgentsSettingsPanel";
import { HostedCommunitiesSettingsCard } from "./HostedCommunitiesSettingsCard";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { ProfileSettingsCard } from "./ProfileSettingsCard";
import { UpdateChecker } from "../UpdateChecker";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { VoiceSettingsCard } from "./VoiceSettingsCard";

export type SettingsSection =
  | "profile"
  | "notifications"
  | "voice"
  | "experimental"
  | "agents"
  | "channel-templates"
  | "compute"
  | "appearance"
  | "shortcuts"
  | "hosted-communities"
  | "community-members"
  | "moderation"
  | "custom-emoji"
  | "local-archive"
  | "mobile"
  | "updates";

export const DEFAULT_SETTINGS_SECTION: SettingsSection = "profile";

const SETTINGS_SECTION_VALUES: readonly SettingsSection[] = [
  "profile",
  "notifications",
  "voice",
  "experimental",
  "agents",
  "channel-templates",
  "compute",
  "appearance",
  "shortcuts",
  "hosted-communities",
  "community-members",
  "moderation",
  "custom-emoji",
  "local-archive",
  "mobile",
  "updates",
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
    value: "voice",
    label: "Voice",
    icon: Volume2,
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
    label: "Channel templates",
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
    value: "hosted-communities",
    label: "Hosted communities",
    icon: MessagesSquare,
  },
  {
    value: "community-members",
    label: "Invites",
    icon: Ticket,
  },
  {
    value: "moderation",
    label: "Moderation",
    icon: ShieldAlert,
  },
  {
    value: "custom-emoji",
    label: "Custom emoji",
    icon: Smile,
    featureGate: "custom-emoji",
  },
  {
    value: "local-archive",
    label: "Local archive",
    icon: Archive,
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
];

function formatThemeLabel(name: string): string {
  return name
    .split("-")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

/**
 * Derive a display label for a paired theme from its light variant name.
 * Strips mode-specific tokens (light, latte, dawn, lotus, ochin, lighter, plus)
 * from any position, handling names like "github-light-default", "light-plus",
 * "material-theme-lighter", and "gruvbox-light-soft".
 */
function pairedThemeLabel(lightName: string): string {
  const modeTokens = new Set([
    "light",
    "latte",
    "dawn",
    "lotus",
    "ochin",
    "lighter",
    "plus",
  ]);
  const parts = lightName.split("-").filter((t) => !modeTokens.has(t));
  // If stripping removed everything (e.g. "light-plus"), fall back to the raw name
  const base = parts.length > 0 ? parts.join("-") : lightName;
  return formatThemeLabel(base);
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
  const darkName = getThemePair(lightName);
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
        darkGradient={darkName ? BUZZ_GRADIENT_STOPS[darkName] : undefined}
        darkVars={darkVars}
        lightGradient={BUZZ_GRADIENT_STOPS[lightName]}
        lightVars={lightVars}
      />
      <span
        className={cn(
          "mt-1.5 w-full truncate text-xs",
          isActive ? "font-medium text-foreground" : "text-muted-foreground",
        )}
      >
        {pairedThemeLabel(lightName)}
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
        sidebarGradient={BUZZ_GRADIENT_STOPS[name]}
        vars={vars}
      />
      <span
        className={cn(
          "mt-1.5 w-full truncate text-xs",
          isActive ? "font-medium text-foreground" : "text-muted-foreground",
        )}
      >
        {formatThemeLabel(name)}
      </span>
    </button>
  );
}

type AppearanceMode = "system" | "light" | "dark";

const APPEARANCE_MODE_OPTIONS = [
  { mode: "system" as const, label: "System", Icon: SunMoon },
  { mode: "light" as const, label: "Light", Icon: Sun },
  { mode: "dark" as const, label: "Dark", Icon: Moon },
] as const;

// Reveal/hide motion for the accent picker: a small translate + opacity fade.
// The picker sits below the theme grid and reads as tucking up behind it, so
// it enters from above (slides *down* into place when a non-Buzz theme reveals
// it) and exits upward (slides up behind the grid when Buzz hides it). No
// height/scale — height collapse clipped the swatches behind the grid's bottom
// fade (the "white bar"). Snappier than the modal 0.2s since this is a small
// settings control, sharing the modal/ProfileSettingsCard easing curve.
const ACCENT_PICKER_TRANSITION = {
  duration: 0.16,
  ease: [0.23, 1, 0.32, 1] as const,
};

function ThemeSettingsCard() {
  const {
    setTheme,
    selectedThemeName,
    themeName,
    isDark,
    accentColor,
    setAccentColor,
    followSystem,
    setFollowSystem,
  } = useTheme();

  // Per-community scoping labels only earn their place when the user is
  // actually in more than one community; with a single community there is
  // nothing to disambiguate.
  const { activeCommunity, communities } = useCommunities();
  const showCommunityScope = communities.length > 1;
  const communityLabel = appearanceCommunityLabel(activeCommunity?.name);

  // Buzz themes pin a neutral accent (GitHub black in light, white in dark),
  // so the accent picker is hidden while a Buzz theme is active. `themeName` is
  // the effective theme, so this also covers System mode resolving to Buzz.
  const buzzThemeSelected = isBuzzTheme(themeName);
  const accentPickerHidden = buzzThemeSelected;
  const shouldReduceMotion = useReducedMotion();

  const previewVarsByTheme = useThemePreviewVars();
  const { pairedLight, lightOnly, darkOnly } = useThemeCategories();

  // Determine the active mode from current state
  const activeMode: AppearanceMode = followSystem
    ? "system"
    : isDark
      ? "dark"
      : "light";

  const [selectedMode, setSelectedMode] = useState<AppearanceMode>(activeMode);
  const [themeStyleExpanded, setThemeStyleExpanded] = useState(false);

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
    if (mode === "system") {
      setFollowSystem(true);
      // If the current theme is unpaired, resolveSystemTheme can't switch it
      // with the OS. Fall back to the first paired theme so System mode works.
      const pair = getThemePair(selectedThemeName as SyntaxThemeName);
      if (!pair && pairedLight.length > 0) {
        setTheme(pairedLight[0]);
      }
    } else {
      setFollowSystem(false);
      // Switch to the counterpart theme when the current theme doesn't match
      // the selected mode. E.g. if the stored theme is light and the user
      // clicks Dark, apply the dark pair so the app immediately reflects the
      // chosen mode. For unpaired themes (no counterpart), fall back to the
      // first available theme in the target mode's list.
      const currentIsLight = LIGHT_THEMES.has(
        selectedThemeName as SyntaxThemeName,
      );
      const needsDark = mode === "dark" && currentIsLight;
      const needsLight = mode === "light" && !currentIsLight;
      if (needsDark || needsLight) {
        const pair = getThemePair(selectedThemeName as SyntaxThemeName);
        if (pair) {
          setTheme(pair);
        } else {
          // Unpaired theme — pick the first theme from the target mode
          const fallback = needsDark ? allDarkThemes[0] : allLightThemes[0];
          if (fallback) {
            setTheme(fallback);
          }
        }
      }
    }
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
  const selectedPairedTheme =
    selectedMode === "system" ? pairedLight.find(isPairActive) : undefined;
  const selectedTheme = selectedThemeName as SyntaxThemeName;
  const selectedPairedDarkTheme = selectedPairedTheme
    ? getThemePair(selectedPairedTheme)
    : undefined;
  const selectedThemeLabel = selectedPairedTheme
    ? pairedThemeLabel(selectedPairedTheme)
    : formatThemeLabel(selectedTheme);
  const selectedThemePreview = selectedPairedTheme ? (
    <SystemPreferencePreviewFrame
      className="h-[112px] w-[168px] shrink-0"
      darkGradient={
        selectedPairedDarkTheme
          ? BUZZ_GRADIENT_STOPS[selectedPairedDarkTheme]
          : undefined
      }
      darkVars={
        selectedPairedDarkTheme ? getVars(selectedPairedDarkTheme) : null
      }
      lightGradient={BUZZ_GRADIENT_STOPS[selectedPairedTheme]}
      lightVars={getVars(selectedPairedTheme)}
    />
  ) : (
    <ThemePreviewFrame
      className="h-[112px] w-[168px] shrink-0"
      sidebarGradient={BUZZ_GRADIENT_STOPS[selectedTheme]}
      vars={getVars(selectedTheme)}
    />
  );
  const themeStyleGrid = (
    <div
      className="px-4 pb-4 pt-1"
      data-testid="theme-style-options"
      id="theme-style-options"
    >
      {/* Theme grid — constrained to ~3 rows, scrolls internally */}
      <div className="relative">
        {/* Top fade */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 top-0 z-10 h-3"
          style={{
            background:
              "linear-gradient(to bottom, hsl(var(--background)), hsl(var(--background) / 0))",
          }}
        />
        {/* Bottom fade — hidden while the accent picker is visible so its
            near-white gradient (Buzz light) can't mask the swatches below it
            (the "white bar"). Kept only when the picker is hidden. */}
        {accentPickerHidden ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-3"
            style={{
              background:
                "linear-gradient(to top, hsl(var(--background)), hsl(var(--background) / 0))",
            }}
          />
        ) : null}
        <div className="max-h-[430px] overflow-y-auto rounded-lg pt-2">
          <div className="flex flex-wrap gap-4 p-1">
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
      </div>
    </div>
  );

  return (
    <section
      className="flex min-h-0 flex-1 flex-col overflow-y-auto"
      data-testid="settings-theme"
    >
      <SettingsSectionHeader
        title="Appearance"
        description="Choose how Buzz looks and feels."
      />

      <SettingsOptionGroupList>
        <SettingsOptionGroup
          data-testid="appearance-theme-card"
          headerAction={
            showCommunityScope && activeCommunity ? (
              <Badge
                className="max-w-56 font-medium normal-case tracking-normal"
                data-testid="appearance-community-badge"
                variant="outline"
              >
                <span className="truncate">{communityLabel}</span>
              </Badge>
            ) : null
          }
          title={
            <>
              Theme
              {showCommunityScope ? (
                <span className="ml-1 font-normal text-muted-foreground">
                  (per community)
                </span>
              ) : null}
            </>
          }
        >
          <SettingsOptionRow data-testid="appearance-color-mode-row">
            <div className="min-w-0">
              <p className="text-sm font-medium">Color mode</p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                Follow your system or choose a light or dark appearance.
              </p>
            </div>
            <SegmentedControl
              indicatorTestId="appearance-color-mode-indicator"
              legend="Color mode"
              onValueChange={handleModeSelect}
              optionTestIdPrefix="appearance-mode"
              options={APPEARANCE_MODE_OPTIONS.map(({ mode, label, Icon }) => ({
                value: mode,
                label,
                Icon,
              }))}
              testId="appearance-color-mode-control"
              value={selectedMode}
            />
          </SettingsOptionRow>

          <SettingsOptionRow data-testid="theme-style-row">
            <div className="min-w-0">
              <p className="text-sm font-medium">Theme style</p>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                Choose the colors used throughout Buzz.
              </p>
            </div>
            <button
              aria-label={`Theme style, ${selectedThemeLabel}`}
              aria-controls="theme-style-options"
              aria-expanded={themeStyleExpanded}
              className="flex h-auto min-w-0 items-center gap-2 rounded-md bg-transparent p-0 text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="theme-style-trigger"
              onClick={() => setThemeStyleExpanded((expanded) => !expanded)}
              type="button"
            >
              <span
                className="shrink-0"
                data-testid="theme-style-selected-preview"
              >
                {selectedThemePreview}
              </span>
              <ChevronDown
                className={cn(
                  "h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200 ease-out motion-reduce:transition-none",
                  themeStyleExpanded && "rotate-180",
                )}
              />
            </button>
          </SettingsOptionRow>

          {shouldReduceMotion ? (
            themeStyleExpanded ? (
              themeStyleGrid
            ) : null
          ) : (
            <AnimatePresence initial={false}>
              {themeStyleExpanded ? (
                <motion.div
                  animate={{ height: "auto", opacity: 1, y: 0 }}
                  className="overflow-hidden"
                  exit={{ height: 0, opacity: 0, y: -6 }}
                  initial={{ height: 0, opacity: 0, y: -6 }}
                  key="theme-style-options"
                  transition={{
                    duration: 0.22,
                    ease: [0.23, 1, 0.32, 1],
                  }}
                >
                  {themeStyleGrid}
                </motion.div>
              ) : null}
            </AnimatePresence>
          )}

          {/* Accent color picker — hidden for Buzz themes (pinned neutral accent).
              Reveal/hide with the translate-up + opacity fade defined by
              ACCENT_PICKER_TRANSITION above. Reduced motion skips the transition
              and just renders/unrenders. */}
          {shouldReduceMotion ? (
            accentPickerHidden ? null : (
              <AccentPickerContent
                accentColor={accentColor}
                isDark={isDark}
                setAccentColor={setAccentColor}
              />
            )
          ) : (
            <AnimatePresence initial={false}>
              {accentPickerHidden ? null : (
                <motion.div
                  animate={{ opacity: 1, y: 0 }}
                  className="will-change-[opacity,transform]"
                  exit={{ opacity: 0, y: -10 }}
                  initial={{ opacity: 0, y: -10 }}
                  key="accent-picker"
                  transition={ACCENT_PICKER_TRANSITION}
                >
                  <AccentPickerContent
                    accentColor={accentColor}
                    isDark={isDark}
                    setAccentColor={setAccentColor}
                  />
                </motion.div>
              )}
            </AnimatePresence>
          )}

          <GlassBackgroundSetting />
          {buzzThemeSelected ? <ProminentActiveTabSetting /> : null}
        </SettingsOptionGroup>

        <SettingsOptionGroup
          data-testid="appearance-preferences-card"
          title="Preferences"
        >
          <ConversationDisplaySettings />
          <LinkPreviewStyleSetting />
          <ThreadLayoutSetting />
        </SettingsOptionGroup>
      </SettingsOptionGroupList>
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
    case "voice":
      return <VoiceSettingsCard />;
    case "experimental":
      return <ExperimentalFeaturesCard />;
    case "agents":
      return <AgentsSettingsPanel />;
    case "channel-templates":
      return <ChannelTemplatesSettingsCard />;
    case "compute":
      return <MeshComputeSettingsCard />;
    case "appearance":
      return <ThemeSettingsCard />;
    case "shortcuts":
      return <KeyboardShortcutsCard />;
    case "hosted-communities":
      return <HostedCommunitiesSettingsCard />;
    case "community-members":
      return (
        <CommunityMembersSettingsCard currentPubkey={props.currentPubkey} />
      );
    case "moderation":
      return <ModerationQueueCard />;
    case "custom-emoji":
      return <CustomEmojiSettingsCard />;
    case "local-archive":
      return <LocalArchiveSettingsCard />;
    case "mobile":
      return <MobilePairingCard currentPubkey={props.currentPubkey} />;
    case "updates":
      return <UpdateChecker />;
    default: {
      const exhaustiveCheck: never = section;
      return exhaustiveCheck;
    }
  }
}
