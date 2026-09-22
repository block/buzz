import * as React from "react";
import type { ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Eye } from "lucide-react";
import { useTranslation } from "@/i18n";
import {
  setThreadViewMode,
  useThreadViewMode,
  type ThreadViewMode,
} from "@/features/channels/lib/threadViewModePreference";
import { useCommunities } from "@/features/communities/useCommunities";
import { AvatarFramingSlider } from "@/features/profile/ui/AnimatedAvatarControls";
import { contrastColorForBackground } from "@/features/profile/ui/ProfileAvatarEditor.utils";
import {
  setLinkPreviewStyle,
  useLinkPreviewStyle,
  type LinkPreviewStyle,
} from "@/shared/lib/linkPreviewStylePreference";
import { isLinuxPlatform } from "@/shared/lib/platform";
import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import { LinkPreviewAttachmentPresentation } from "@/shared/ui/link-preview-attachment";
import type { LinkPreviewImageLightboxProps } from "@/shared/ui/rich-link-preview-attachment";
import {
  previewConversationDensity,
  setConversationDensity,
  useConversationDensity,
  type ConversationDensity,
} from "@/shared/lib/conversationDensityPreference";
import {
  previewFontSize,
  setFontSize,
  useFontSize,
  type FontSize,
} from "@/shared/lib/fontSizePreference";
import {
  ACCENT_COLORS,
  DEFAULT_GLASS_OPACITY,
  GLASS_OPACITY_MAX,
  GLASS_OPACITY_MIN,
  NEUTRAL_ACCENT,
  useTheme,
} from "@/shared/theme/ThemeProvider";

import { Switch } from "@/shared/ui/switch";
import { SettingsOptionRow } from "./SettingsOptionGroup";
import { SegmentedControl } from "@/shared/ui/segmented-control";

/** Buzz navigation can use either its production tint or a stronger tab. */
export function ProminentActiveTabSetting() {
  const { t } = useTranslation();
  const { prominentActiveTab, setProminentActiveTab } = useTheme();

  return (
    <SettingsOptionRow data-testid="prominent-active-tab-row">
      <div className="min-w-0">
        <label
          className="text-sm font-medium"
          htmlFor="prominent-active-tab-switch"
        >
          {t("settings.appearance.prominent-tab.label")}
        </label>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          {t("settings.appearance.prominent-tab.hint")}
        </p>
      </div>
      <Switch
        checked={prominentActiveTab}
        data-testid="prominent-active-tab-toggle"
        id="prominent-active-tab-switch"
        onCheckedChange={setProminentActiveTab}
      />
    </SettingsOptionRow>
  );
}

/** Style ids in picker order; labels and hints resolve at render via `t()`. */
const LINK_PREVIEW_STYLE_OPTIONS: { value: LinkPreviewStyle }[] = [
  { value: "compact" },
  { value: "rich" },
];

const CONVERSATION_DENSITY_OPTIONS: readonly {
  value: ConversationDensity;
}[] = [{ value: "compact" }, { value: "comfortable" }, { value: "spacious" }];

function ConversationDensityPreviewMessage({
  avatar,
  author,
  children,
  timestamp,
}: {
  avatar: string;
  author: string;
  children: ReactNode;
  timestamp: string;
}) {
  return (
    <article className="flex gap-2.5 py-conversation-row">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold text-muted-foreground">
        {avatar}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 leading-message-author">
          <span className="text-message font-semibold leading-message-author tracking-normal text-foreground">
            {author}
          </span>
          <span className="text-message-timestamp font-normal text-muted-foreground/65">
            {timestamp}
          </span>
        </div>
        <div className="mt-conversation-body text-message font-normal tracking-normal text-foreground">
          {children}
        </div>
      </div>
    </article>
  );
}

function ConversationPreview() {
  const { t } = useTranslation();
  return (
    <div className="px-4 py-3" data-testid="conversation-preview">
      <div
        aria-hidden="true"
        className="relative overflow-hidden rounded-xl border border-border/65 bg-transparent"
        data-testid="conversation-preview-surface"
      >
        <span className="absolute right-3.5 top-3 inline-flex items-center gap-1 text-2xs font-medium text-muted-foreground/55">
          <Eye aria-hidden="true" className="size-3" />
          {t("settings.appearance.preview.label")}
        </span>
        <div className="p-4" data-testid="conversation-preview-content">
          <ConversationDensityPreviewMessage
            avatar="M"
            author="Maya"
            timestamp="9:41"
          >
            {t("settings.appearance.preview.sample-message-1")}
          </ConversationDensityPreviewMessage>
          <ConversationDensityPreviewMessage
            avatar="T"
            author="Theo"
            timestamp="9:43"
          >
            <p>{t("settings.appearance.preview.sample-message-2")}</p>
            <p className="mt-conversation-paragraph">
              {t("settings.appearance.preview.sample-message-3")}
            </p>
          </ConversationDensityPreviewMessage>
        </div>
      </div>
    </div>
  );
}

/** App-wide type sizing and conversation-specific spacing controls. */
export function ConversationDisplaySettings() {
  const { t } = useTranslation();
  const density = useConversationDensity();
  const fontSize = useFontSize();

  const fontSizeOptions: readonly { value: FontSize; label: string }[] = [
    {
      value: "smaller",
      label: t("settings.appearance.font.size.option.smaller"),
    },
    {
      value: "default",
      label: t("settings.appearance.font.size.option.default"),
    },
    {
      value: "larger",
      label: t("settings.appearance.font.size.option.larger"),
    },
  ];

  const densityOptions: { value: ConversationDensity; label: string }[] =
    CONVERSATION_DENSITY_OPTIONS.map((option) => ({
      value: option.value,
      label:
        option.value === "compact"
          ? t("settings.appearance.density.option-compact")
          : option.value === "comfortable"
            ? t("settings.appearance.density.option-comfy")
            : t("settings.appearance.density.option-spacious"),
    }));

  return (
    <div data-testid="conversation-display-group">
      <SettingsOptionRow data-testid="font-size-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">
            {t("settings.appearance.font.size.label")}
          </p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {t("settings.appearance.font.size.description")}
          </p>
        </div>
        <SegmentedControl
          size="wide"
          legend={t("settings.appearance.font.size.label")}
          onPreviewChange={previewFontSize}
          onValueChange={setFontSize}
          optionTestIdPrefix="font-size"
          options={fontSizeOptions}
          testId="font-size-control"
          value={fontSize}
        />
      </SettingsOptionRow>
      <SettingsOptionRow data-testid="conversation-density-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">
            {t("settings.appearance.density.label")}
          </p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {t("settings.appearance.density.hint")}
          </p>
        </div>
        <SegmentedControl
          size="wide"
          legend={t("settings.appearance.density.label")}
          onPreviewChange={previewConversationDensity}
          onValueChange={setConversationDensity}
          optionTestIdPrefix="conversation-density"
          options={densityOptions}
          testId="conversation-density-control"
          value={density}
        />
      </SettingsOptionRow>
      <ConversationPreview />
    </div>
  );
}

/**
 * Static sample used by the settings preview card. The thumbnail is an inline
 * SVG data URL so the preview needs no network fetch or native image pipeline.
 * The sample's title and description are UI copy, so `LinkPreviewSample`
 * supplies them per render from the catalog.
 */
const LINK_PREVIEW_SAMPLE_BASE: Omit<
  ResolvedLinkPreview,
  "imageDataUrl" | "title" | "description"
> = {
  kind: "generic-link",
  href: "https://example.com/product-updates",
  provider: "example.com",
  typeLabel: "link",
  imageState: "image",
  imageDomain: "example.com",
};

/**
 * Build the sample thumbnail as an SVG data URL from the Buzz gradient
 * tokens. Data-URL images cannot resolve CSS variables, so the token values
 * are read from the live stylesheet and baked in per render — if the Buzz
 * gradient ever changes in `theme.css`, this preview follows automatically.
 */
function buzzGradientSampleImage(isDark: boolean): string {
  const styles = globalThis.document
    ? getComputedStyle(document.documentElement)
    : null;
  const readToken = (token: string, fallback: string): string =>
    styles?.getPropertyValue(token).trim() || fallback;
  const top = isDark
    ? readToken("--buzz-gradient-dark-top", "#4a4616")
    : readToken("--buzz-gradient-light-top", "#e6e6b6");
  const bottom = isDark
    ? readToken("--buzz-gradient-dark-bottom", "#0a1423")
    : readToken("--buzz-gradient-light-bottom", "#c4d0da");
  const shapeToken = isDark ? "--foreground" : "--background";
  const shapeFallback = isDark ? "0 0% 98%" : "0 0% 100%";
  const shape = `hsl(${readToken(shapeToken, shapeFallback)})`;
  const shapeOpacities = isDark ? [0.5, 0.38, 0.28] : [0.82, 0.68, 0.52];
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 382 200"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${top}"/><stop offset="1" stop-color="${bottom}"/></linearGradient></defs><rect width="382" height="200" fill="url(#g)"/><rect x="76" y="64" width="72" height="72" rx="22" fill="${shape}" opacity="${shapeOpacities[0]}"/><rect x="168" y="76" width="96" height="18" rx="9" fill="${shape}" opacity="${shapeOpacities[1]}"/><rect x="168" y="106" width="138" height="18" rx="9" fill="${shape}" opacity="${shapeOpacities[2]}"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

/** Lightbox stand-in for the settings sample — renders the image inert. */
function SampleImageLightbox({
  children,
  className,
}: LinkPreviewImageLightboxProps) {
  return <div className={className}>{children}</div>;
}

function LinkPreviewSample({ style }: { style: LinkPreviewStyle }) {
  const { t } = useTranslation();
  const { isDark } = useTheme();
  const preview = React.useMemo<ResolvedLinkPreview>(
    () => ({
      ...LINK_PREVIEW_SAMPLE_BASE,
      title: t("settings.appearance.preview.sample-link-title"),
      description: t("settings.appearance.preview.sample-link-description"),
      imageDataUrl: buzzGradientSampleImage(isDark),
    }),
    [isDark, t],
  );
  return (
    <div className="px-4 py-3" data-testid="link-preview-sample">
      <div
        aria-hidden="true"
        className="relative overflow-hidden rounded-xl border border-border/65 bg-transparent"
        data-testid="link-preview-sample-surface"
        inert
      >
        <span className="absolute right-3.5 top-3 inline-flex items-center gap-1 text-2xs font-medium text-muted-foreground/55">
          <Eye aria-hidden="true" className="size-3" />
          {t("settings.appearance.preview.label")}
        </span>
        <div className="p-4 pr-24">
          <LinkPreviewAttachmentPresentation
            ImageLightbox={SampleImageLightbox}
            preview={preview}
            showExpandControl={false}
            style={style}
          />
        </div>
      </div>
    </div>
  );
}

export function LinkPreviewStyleSetting() {
  const { t } = useTranslation();
  const style = useLinkPreviewStyle();
  const [previewStyle, setPreviewStyle] =
    React.useState<LinkPreviewStyle | null>(null);
  const displayedStyle = previewStyle ?? style;

  const styleLabel = (value: LinkPreviewStyle) =>
    value === "rich"
      ? t("shared.linkPreview.style.rich")
      : t("shared.linkPreview.style.compact");

  const styleOptions = LINK_PREVIEW_STYLE_OPTIONS.map((option) => ({
    value: option.value,
    label: styleLabel(option.value),
  }));

  return (
    <div data-testid="link-preview-style-group">
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">
            {t("settings.appearance.link-previews.label")}
          </p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {displayedStyle === "rich"
              ? t("settings.appearance.link-previews.rich-hint")
              : t("settings.appearance.link-previews.compact-hint")}
          </p>
        </div>
        <SegmentedControl
          size="compact"
          legend={t("settings.appearance.link-previews.label")}
          onPreviewChange={setPreviewStyle}
          onValueChange={setLinkPreviewStyle}
          optionTestIdPrefix="link-preview-style"
          options={styleOptions}
          testId="link-preview-style-control"
          value={style}
        />
      </SettingsOptionRow>
      <LinkPreviewSample style={displayedStyle} />
    </div>
  );
}

/** Thread modes in picker order; labels and hints resolve via `t()`. */
const THREAD_VIEW_MODE_OPTIONS: { value: ThreadViewMode }[] = [
  { value: "focus" },
  { value: "split" },
];

/** Native window glass rows sit below theme and accent choices. */
export function GlassBackgroundSetting() {
  const { t } = useTranslation();
  const {
    glassBackground,
    glassBackgroundSupported,
    glassOpacity,
    setGlassBackground,
    setGlassOpacity,
  } = useTheme();
  const shouldReduceMotion = useReducedMotion();

  if (isLinuxPlatform()) return null;

  const shouldShowOpacity = glassBackgroundSupported && glassBackground;
  const opacityRow = (
    <SettingsOptionRow data-testid="glass-opacity-row">
      <div className="min-w-0">
        <p className="text-sm font-medium">
          {t("settings.appearance.glass.opacity-label")}
        </p>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
          id="glass-opacity-description"
        >
          {t("settings.appearance.glass.opacity-hint")}
        </p>
      </div>
      <div className="flex w-64 shrink-0 items-center">
        <AvatarFramingSlider
          ariaDescribedBy="glass-opacity-description"
          ariaLabel={t("settings.appearance.glass.opacity-label")}
          ariaValueText={`${glassOpacity}% opacity`}
          compact
          handleAlwaysVisible
          max={GLASS_OPACITY_MAX}
          min={GLASS_OPACITY_MIN}
          onChange={setGlassOpacity}
          onReset={() => setGlassOpacity(DEFAULT_GLASS_OPACITY)}
          resetLabel="Reset glass opacity"
          resetTestId="glass-opacity-reset"
          resetValue={DEFAULT_GLASS_OPACITY}
          testId="glass-opacity-slider"
          value={glassOpacity}
        />
      </div>
    </SettingsOptionRow>
  );

  return (
    <>
      <SettingsOptionRow data-testid="glass-background-row">
        <div className="min-w-0">
          <label
            className="text-sm font-medium"
            htmlFor="glass-background-switch"
          >
            {t("settings.appearance.glass.label")}
          </label>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {glassBackgroundSupported
              ? t("settings.appearance.glass.hint")
              : t("settings.appearance.glass.unsupported-hint")}
          </p>
        </div>
        <Switch
          checked={glassBackgroundSupported && glassBackground}
          data-testid="glass-background-toggle"
          disabled={!glassBackgroundSupported}
          id="glass-background-switch"
          onCheckedChange={setGlassBackground}
        />
      </SettingsOptionRow>
      {shouldReduceMotion ? (
        shouldShowOpacity ? (
          opacityRow
        ) : null
      ) : (
        <AnimatePresence initial={false}>
          {shouldShowOpacity ? (
            <motion.div
              animate={{ height: "auto", opacity: 1, y: 0 }}
              className="overflow-hidden"
              exit={{ height: 0, opacity: 0, y: -6 }}
              initial={{ height: 0, opacity: 0, y: -6 }}
              key="glass-opacity"
              transition={{
                duration: 0.25,
                ease: [0.23, 1, 0.32, 1],
              }}
            >
              {opacityRow}
            </motion.div>
          ) : null}
        </AnimatePresence>
      )}
    </>
  );
}

/** Compact thread preference row in the Appearance preferences card. */
/**
 * Abstract diagram for the thread layout preview, in the same soft-block
 * style as the links sample: a rounded frame holding a channel surface and a
 * thread surface, with light skeleton bars. Inline SVG (not a data-URL image)
 * so fills reference theme tokens directly and follow light/dark and accent
 * changes automatically. Only the panel proportions change between modes.
 */
function ThreadLayoutDiagram({ mode }: { mode: ThreadViewMode }) {
  const { isDark } = useTheme();
  const gradientId = React.useId();
  // Inline SVG resolves CSS variables, so the frame gradient references the
  // Buzz gradient tokens directly and follows theme.css automatically.
  const gradientTop = isDark
    ? "var(--buzz-gradient-dark-top, #4a4616)"
    : "var(--buzz-gradient-light-top, #e6e6b6)";
  const gradientBottom = isDark
    ? "var(--buzz-gradient-dark-bottom, #0a1423)"
    : "var(--buzz-gradient-light-bottom, #c4d0da)";
  const channelSurface = "hsl(var(--muted))";
  const threadSurface = "hsl(var(--background))";
  const channelOpacity = isDark ? 0.88 : 0.78;
  const threadOpacity = isDark ? 0.98 : 0.96;
  const bar = "hsl(var(--foreground) / 0.24)";
  const barSoft = "hsl(var(--foreground) / 0.14)";

  const isFocus = mode === "focus";
  // Inner content area: 10..230 x 10..122 (inside the frame padding).
  // Split: channel and thread share the area side by side with a gap.
  // Focus: the channel continues beneath the overlaid thread, leaving only
  // a narrow orientation sliver visible at the left edge.
  const gap = 6;
  const threadX = isFocus ? 42 : 124;
  const channelWidth = isFocus ? 64 : threadX - 10 - gap;
  const threadWidth = 230 - threadX;

  /** Two skeleton text bars, clipped to the panel they sit in. */
  const skeleton = (x: number, y: number, width: number) => (
    <>
      <rect fill={bar} height={7} rx={3.5} width={width * 0.62} x={x} y={y} />
      <rect
        fill={barSoft}
        height={7}
        rx={3.5}
        width={width * 0.86}
        x={x}
        y={y + 13}
      />
    </>
  );

  return (
    <svg
      aria-hidden="true"
      className="block w-full max-w-60"
      data-testid={`thread-layout-diagram-${mode}`}
      role="img"
      viewBox="0 0 240 132"
    >
      <defs>
        <linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor={gradientTop} />
          <stop offset="1" stopColor={gradientBottom} />
        </linearGradient>
      </defs>
      {/* Frame */}
      <rect fill={`url(#${gradientId})`} height={132} rx={18} width={240} />
      {/* Channel surface */}
      <rect
        fill={channelSurface}
        height={112}
        opacity={channelOpacity}
        rx={10}
        width={channelWidth}
        x={10}
        y={10}
      />
      {channelWidth > 60 ? skeleton(22, 24, channelWidth - 24) : null}
      {/* Thread surface */}
      <path
        d={`M ${threadX + 10} 10 H 220 Q 230 10 230 20 V 112 Q 230 122 220 122 H ${threadX + 10} Q ${threadX} 122 ${threadX} 112 V 20 Q ${threadX} 10 ${threadX + 10} 10 Z`}
        fill={threadSurface}
        opacity={threadOpacity}
      />
      {skeleton(threadX + 12, 24, threadWidth - 24)}
    </svg>
  );
}

function ThreadLayoutPreview({ mode }: { mode: ThreadViewMode }) {
  const { t } = useTranslation();
  return (
    <div className="px-4 py-3" data-testid="thread-layout-preview">
      <div
        aria-hidden="true"
        className="relative overflow-hidden rounded-xl border border-border/65 bg-transparent"
        data-testid="thread-layout-preview-surface"
      >
        <span className="absolute right-3.5 top-3 inline-flex items-center gap-1 text-2xs font-medium text-muted-foreground/55">
          <Eye aria-hidden="true" className="size-3" />
          {t("settings.appearance.preview.label")}
        </span>
        <div className="p-4 pr-24">
          <ThreadLayoutDiagram mode={mode} />
        </div>
      </div>
    </div>
  );
}

export function ThreadLayoutSetting() {
  const { t } = useTranslation();
  const threadViewMode = useThreadViewMode();
  const [previewMode, setPreviewMode] = React.useState<ThreadViewMode | null>(
    null,
  );
  const { communities } = useCommunities();
  const showCommunityScope = communities.length > 1;
  const displayedMode = previewMode ?? threadViewMode;
  const threadOptions = THREAD_VIEW_MODE_OPTIONS.map((option) => ({
    value: option.value,
    label:
      option.value === "focus"
        ? t("settings.appearance.thread-layout.option-focus")
        : t("settings.appearance.thread-layout.option-split"),
  }));

  return (
    <div data-testid="thread-layout-group">
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">
            {t("settings.appearance.thread-layout.label")}
            {showCommunityScope ? (
              <span className="font-normal text-muted-foreground">
                {" "}
                {t("settings.appearance.thread-layout.scope")}
              </span>
            ) : null}
          </p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {displayedMode === "focus"
              ? t("settings.appearance.thread-layout.focus-hint")
              : t("settings.appearance.thread-layout.split-hint")}
          </p>
        </div>
        <SegmentedControl
          size="compact"
          legend={t("settings.appearance.thread-layout.label")}
          onPreviewChange={setPreviewMode}
          onValueChange={setThreadViewMode}
          optionTestIdPrefix="thread-layout"
          options={threadOptions}
          testId="thread-layout-control"
          value={threadViewMode}
        />
      </SettingsOptionRow>
      <ThreadLayoutPreview mode={displayedMode} />
    </div>
  );
}

/** Accent swatches — shared by the animated and reduced-motion reveal paths. */
export function AccentPickerContent({
  accentColor,
  isDark,
  setAccentColor,
}: {
  accentColor: string;
  isDark: boolean;
  setAccentColor: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <SettingsOptionRow className="items-start">
      <div className="min-w-0">
        <p className="text-sm font-medium">
          {t("settings.appearance.accent.label")}
        </p>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          {t("settings.appearance.accent.hint")}
        </p>
      </div>
      <div
        className="min-w-0 max-w-[34rem] shrink-0 overflow-x-auto rounded-xl bg-muted p-2"
        data-testid="accent-color-options"
      >
        <div className="flex w-max min-w-full flex-nowrap justify-end gap-2">
          {ACCENT_COLORS.map((color) => {
            const isNeutral = color.value === NEUTRAL_ACCENT;
            const isSelected = accentColor === color.value;
            const swatchColor = isNeutral
              ? "hsl(var(--foreground))"
              : color.value;
            const selectionColor = isNeutral
              ? isDark
                ? "#000000"
                : "#FFFFFF"
              : contrastColorForBackground(color.value);

            return (
              <button
                aria-label={t("settings.appearance.accent.swatch-aria", {
                  name: color.name,
                })}
                aria-pressed={isSelected}
                className="relative h-9 w-9 shrink-0 rounded-full border border-border transition-transform duration-200 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transform-none motion-reduce:transition-none"
                data-testid={`accent-color-${color.name.toLowerCase()}`}
                key={color.value}
                onClick={() => setAccentColor(color.value)}
                style={{ backgroundColor: swatchColor }}
                title={color.name}
                type="button"
              >
                {isSelected ? (
                  <span
                    className="absolute inset-1 rounded-full border-[3px]"
                    data-testid="accent-color-selection"
                    style={{ borderColor: selectionColor }}
                  />
                ) : null}
              </button>
            );
          })}
        </div>
      </div>
    </SettingsOptionRow>
  );
}
