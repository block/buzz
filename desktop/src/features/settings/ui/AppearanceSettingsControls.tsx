import * as React from "react";
import type { ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Hash, Inbox, MessageSquare, Search, X } from "lucide-react";
import {
  setThreadViewMode,
  useThreadViewMode,
  type ThreadViewMode,
} from "@/features/channels/lib/threadViewModePreference";
import { useCommunities } from "@/features/communities/useCommunities";
import { AvatarFramingSlider } from "@/features/profile/ui/AnimatedAvatarControls";
import { cn } from "@/shared/lib/cn";
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
  const { prominentActiveTab, setProminentActiveTab } = useTheme();

  return (
    <SettingsOptionRow data-testid="prominent-active-tab-row">
      <div className="min-w-0">
        <label
          className="text-sm font-medium"
          htmlFor="prominent-active-tab-switch"
        >
          Prominent active tab
        </label>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          Give the selected navigation item a higher-contrast background.
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

const LINK_PREVIEW_STYLE_OPTIONS: {
  value: LinkPreviewStyle;
  label: string;
  description: string;
}[] = [
  {
    value: "compact",
    label: "Compact",
    description: "Small cards with a thumbnail",
  },
  {
    value: "rich",
    label: "Rich",
    description: "Large previews with images and descriptions",
  },
];

const CONVERSATION_DENSITY_OPTIONS: readonly {
  value: ConversationDensity;
  label: string;
}[] = [
  {
    value: "compact",
    label: "Compact",
  },
  {
    value: "comfortable",
    label: "Comfy",
  },
  {
    value: "spacious",
    label: "Spacious",
  },
];

const FONT_SIZE_OPTIONS: readonly {
  value: FontSize;
  label: string;
}[] = [
  {
    value: "smaller",
    label: "Smaller",
  },
  {
    value: "default",
    label: "Default",
  },
  {
    value: "larger",
    label: "Larger",
  },
];

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

/** App-wide type sizing and conversation-specific spacing controls. */
export function ConversationDisplaySettings() {
  const density = useConversationDensity();
  const fontSize = useFontSize();

  return (
    <div data-testid="conversation-display-group">
      <SettingsOptionRow data-testid="font-size-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">Font size</p>
        </div>
        <SegmentedControl
          size="wide"
          legend="Font size"
          onPreviewChange={previewFontSize}
          onValueChange={setFontSize}
          optionTestIdPrefix="font-size"
          options={FONT_SIZE_OPTIONS}
          testId="font-size-control"
          value={fontSize}
        />
      </SettingsOptionRow>
      <SettingsOptionRow
        className="border-t border-border/55"
        data-testid="conversation-density-row"
      >
        <div className="min-w-0">
          <p className="text-sm font-medium">Conversation density</p>
        </div>
        <SegmentedControl
          size="wide"
          legend="Conversation density"
          onPreviewChange={previewConversationDensity}
          onValueChange={setConversationDensity}
          optionTestIdPrefix="conversation-density"
          options={CONVERSATION_DENSITY_OPTIONS}
          testId="conversation-density-control"
          value={density}
        />
      </SettingsOptionRow>
    </div>
  );
}

/**
 * Static sample used by the settings preview card. The thumbnail is an inline
 * SVG data URL so the preview needs no network fetch or native image pipeline.
 */
const LINK_PREVIEW_SAMPLE_BASE: Omit<ResolvedLinkPreview, "imageDataUrl"> = {
  kind: "generic-link",
  href: "https://example.com/product-updates",
  provider: "example.com",
  title: "Product updates — a fresh look at conversations",
  typeLabel: "link",
  description:
    "Highlights from this release: refreshed conversation layout, quicker link handling, and readability improvements.",
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

const APPEARANCE_PREVIEW_EASE_OUT = [0.23, 1, 0.32, 1] as const;
const APPEARANCE_PREVIEW_EASE_IN_OUT = [0.77, 0, 0.175, 1] as const;
const APPEARANCE_PREVIEW_MORPH_TRANSITION = {
  duration: 0.24,
  ease: APPEARANCE_PREVIEW_EASE_IN_OUT,
} as const;
const APPEARANCE_PREVIEW_CROSSFADE_TRANSITION = {
  duration: 0.18,
  ease: APPEARANCE_PREVIEW_EASE_OUT,
} as const;
const APPEARANCE_PREVIEW_REDUCED_TRANSITION = {
  duration: 0.12,
  ease: "linear",
} as const;
const APPEARANCE_PREVIEW_INSTANT_TRANSITION = { duration: 0 } as const;
const APPEARANCE_PREVIEW_BUTTON_MOTION =
  "transition-[color,background-color,transform] duration-[var(--motion-duration-instant)] ease-[var(--motion-ease-out)] active:scale-[0.97] motion-reduce:transform-none motion-reduce:transition-colors";

type AppearancePreviewMotionContext = {
  direct: boolean;
  modeChanged: boolean;
  reduced: boolean;
};

function appearancePreviewTransition({
  direct,
  reduced,
}: AppearancePreviewMotionContext) {
  if (direct) return APPEARANCE_PREVIEW_INSTANT_TRANSITION;
  if (reduced) return APPEARANCE_PREVIEW_REDUCED_TRANSITION;
  return APPEARANCE_PREVIEW_MORPH_TRANSITION;
}

const APPEARANCE_PREVIEW_PANEL_VARIANTS = {
  animate: (context: AppearancePreviewMotionContext) => ({
    filter: "blur(0px)",
    opacity: 1,
    transform: "translateX(0)",
    transition: appearancePreviewTransition(context),
  }),
  exit: (context: AppearancePreviewMotionContext) => ({
    filter:
      !context.direct && !context.reduced && context.modeChanged
        ? "blur(2px)"
        : "blur(0px)",
    opacity: context.direct ? 1 : 0,
    transform:
      context.direct || context.reduced || context.modeChanged
        ? "translateX(0)"
        : "translateX(100%)",
    transition: appearancePreviewTransition(context),
  }),
  initial: (context: AppearancePreviewMotionContext) => ({
    filter:
      !context.direct && !context.reduced && context.modeChanged
        ? "blur(2px)"
        : "blur(0px)",
    opacity: context.direct ? 1 : 0,
    transform:
      context.direct || context.reduced || context.modeChanged
        ? "translateX(0)"
        : "translateX(100%)",
    transition: appearancePreviewTransition(context),
  }),
} as const;

const APPEARANCE_PREVIEW_CONTENT_VARIANTS = {
  animate: (context: AppearancePreviewMotionContext) => ({
    filter: "blur(0px)",
    opacity: 1,
    transition: context.direct
      ? APPEARANCE_PREVIEW_INSTANT_TRANSITION
      : context.reduced
        ? APPEARANCE_PREVIEW_REDUCED_TRANSITION
        : APPEARANCE_PREVIEW_CROSSFADE_TRANSITION,
  }),
  exit: (context: AppearancePreviewMotionContext) => ({
    filter: context.direct || context.reduced ? "blur(0px)" : "blur(2px)",
    opacity: context.direct ? 1 : 0,
    transition: context.direct
      ? APPEARANCE_PREVIEW_INSTANT_TRANSITION
      : context.reduced
        ? APPEARANCE_PREVIEW_REDUCED_TRANSITION
        : APPEARANCE_PREVIEW_CROSSFADE_TRANSITION,
  }),
  initial: (context: AppearancePreviewMotionContext) => ({
    filter: context.direct || context.reduced ? "blur(0px)" : "blur(2px)",
    opacity: context.direct ? 1 : 0,
    transition: context.direct
      ? APPEARANCE_PREVIEW_INSTANT_TRANSITION
      : context.reduced
        ? APPEARANCE_PREVIEW_REDUCED_TRANSITION
        : APPEARANCE_PREVIEW_CROSSFADE_TRANSITION,
  }),
} as const;

export function AppearanceWorkspacePreview({
  previewLinkStyle,
  previewThreadMode,
}: {
  previewLinkStyle?: LinkPreviewStyle | null;
  previewThreadMode?: ThreadViewMode | null;
}) {
  const savedLinkStyle = useLinkPreviewStyle();
  const savedThreadMode = useThreadViewMode();
  const { glassBackground, isDark, prominentActiveTab } = useTheme();
  const shouldReduceMotion = useReducedMotion();
  const [activeChannel, setActiveChannel] = React.useState<
    "design" | "general"
  >("design");
  const [threadOpen, setThreadOpen] = React.useState(true);
  const linkStyle = previewLinkStyle ?? savedLinkStyle;
  const threadMode = previewThreadMode ?? savedThreadMode;
  React.useEffect(() => {
    if (previewThreadMode !== null && previewThreadMode !== undefined) {
      setThreadOpen(true);
    } else if (previewLinkStyle !== null && previewLinkStyle !== undefined) {
      setThreadOpen(false);
    }
  }, [previewLinkStyle, previewThreadMode]);
  const previousThreadOpenRef = React.useRef(threadOpen);
  const threadVisibilityChanged = previousThreadOpenRef.current !== threadOpen;
  React.useEffect(() => {
    previousThreadOpenRef.current = threadOpen;
  }, [threadOpen]);
  const directManipulationActive =
    !threadVisibilityChanged &&
    ((previewLinkStyle !== null && previewLinkStyle !== undefined) ||
      (previewThreadMode !== null && previewThreadMode !== undefined));
  const previousThreadModeRef = React.useRef(threadMode);
  const threadModeChanged = previousThreadModeRef.current !== threadMode;
  React.useEffect(() => {
    previousThreadModeRef.current = threadMode;
  }, [threadMode]);
  const preview = React.useMemo<ResolvedLinkPreview>(
    () => ({
      ...LINK_PREVIEW_SAMPLE_BASE,
      imageDataUrl: buzzGradientSampleImage(isDark),
    }),
    [isDark],
  );

  const navItemClass = (active: boolean) =>
    cn(
      "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs",
      APPEARANCE_PREVIEW_BUTTON_MOTION,
      active
        ? prominentActiveTab
          ? "bg-primary text-primary-foreground"
          : "bg-accent text-accent-foreground"
        : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
    );

  const motionMode = directManipulationActive
    ? "direct"
    : shouldReduceMotion
      ? "reduced"
      : "animated";
  const motionContext: AppearancePreviewMotionContext = {
    direct: directManipulationActive,
    modeChanged: threadModeChanged,
    reduced: Boolean(shouldReduceMotion),
  };

  const threadPanel = (
    <motion.aside
      animate="animate"
      className={cn(
        "absolute inset-y-0 right-0 z-10 flex min-w-0 flex-col overflow-hidden border-l border-border/60 bg-background will-change-[opacity,transform]",
        threadMode === "split" ? "w-[42%]" : "w-[72%]",
      )}
      custom={motionContext}
      data-motion-mode={motionMode}
      data-testid="appearance-workspace-preview-thread"
      exit="exit"
      initial="initial"
      key={threadMode}
      variants={APPEARANCE_PREVIEW_PANEL_VARIANTS}
    >
      <div className="flex h-11 shrink-0 items-center justify-between border-b border-border/60 px-3">
        <div className="min-w-0">
          <p className="truncate text-xs font-semibold">Layout feedback</p>
          <p className="text-2xs text-muted-foreground">3 replies</p>
        </div>
        <button
          aria-label="Close preview thread"
          className={cn(
            "rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground",
            APPEARANCE_PREVIEW_BUTTON_MOTION,
          )}
          data-testid="appearance-preview-close-thread"
          onClick={() => setThreadOpen(false)}
          type="button"
        >
          <X className="size-3.5" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden px-3 py-1">
        <ConversationDensityPreviewMessage
          avatar="M"
          author="Maya"
          timestamp="9:41"
        >
          The revised conversation layout is ready to review.
        </ConversationDensityPreviewMessage>
        <ConversationDensityPreviewMessage
          avatar="A"
          author="Arjun"
          timestamp="9:44"
        >
          This hierarchy feels much clearer.
        </ConversationDensityPreviewMessage>
      </div>
      <div className="m-3 rounded-lg border border-border/70 bg-muted/35 px-3 py-2 text-2xs text-muted-foreground">
        Reply to thread…
      </div>
    </motion.aside>
  );

  const channelLayout = threadOpen && threadMode === "split" ? "split" : "full";
  const channelContentKey = `${activeChannel}:${linkStyle}:${channelLayout}`;
  const firstMessage =
    activeChannel === "design"
      ? "The revised conversation layout is ready to review."
      : "The weekly project notes are ready for everyone.";
  const secondMessage =
    activeChannel === "design"
      ? "I also updated how shared links appear in the conversation."
      : "I linked the release notes so the team can catch up quickly.";

  const channelPane = (
    <motion.main
      animate="animate"
      className={cn(
        "absolute inset-y-0 left-0 flex min-w-0 flex-col overflow-hidden will-change-[opacity]",
        channelLayout === "split" ? "right-[42%]" : "right-0",
      )}
      custom={motionContext}
      data-motion-mode={motionMode}
      data-testid="appearance-workspace-preview-channel"
      exit="exit"
      initial="initial"
      key={channelContentKey}
      variants={APPEARANCE_PREVIEW_CONTENT_VARIANTS}
    >
      <header className="flex h-11 shrink-0 items-center justify-between border-b border-border/60 px-3">
        <div className="flex min-w-0 items-center gap-1.5">
          <Hash className="size-3.5 text-muted-foreground" />
          <span className="truncate text-xs font-semibold">
            {activeChannel}
          </span>
        </div>
        {!threadOpen ? (
          <button
            className={cn(
              "flex items-center gap-1 rounded-md px-1.5 py-1 text-2xs text-muted-foreground hover:bg-muted hover:text-foreground",
              APPEARANCE_PREVIEW_BUTTON_MOTION,
            )}
            data-testid="appearance-preview-open-thread"
            onClick={() => setThreadOpen(true)}
            type="button"
          >
            <MessageSquare className="size-3" /> Open thread
          </button>
        ) : null}
      </header>
      <div
        className="min-h-0 flex-1 overflow-hidden px-3 py-1"
        data-testid="appearance-workspace-preview-messages"
      >
        <button
          className={cn(
            "block w-full rounded-lg text-left hover:bg-muted/35",
            APPEARANCE_PREVIEW_BUTTON_MOTION,
          )}
          data-testid="appearance-preview-thread-message"
          onClick={() => setThreadOpen(true)}
          type="button"
        >
          <ConversationDensityPreviewMessage
            avatar="M"
            author="Maya"
            timestamp="9:41"
          >
            {firstMessage}
          </ConversationDensityPreviewMessage>
        </button>
        <ConversationDensityPreviewMessage
          avatar="T"
          author="Theo"
          timestamp="9:43"
        >
          {secondMessage}
          <div
            className="mt-2 max-w-[300px] origin-top-left scale-[0.78]"
            data-testid="appearance-workspace-link-preview"
            inert
          >
            <LinkPreviewAttachmentPresentation
              ImageLightbox={SampleImageLightbox}
              preview={preview}
              showExpandControl={false}
              style={linkStyle}
            />
          </div>
        </ConversationDensityPreviewMessage>
      </div>
      <div className="m-3 rounded-lg border border-border/70 bg-muted/35 px-3 py-2 text-2xs text-muted-foreground">
        Message #{activeChannel}
      </div>
    </motion.main>
  );

  return (
    <div className="mb-10" data-testid="appearance-workspace-preview">
      <div className="mb-2 flex items-baseline px-4">
        <p className="text-sm font-semibold text-muted-foreground/70">
          Preview
        </p>
      </div>
      <div
        className={cn(
          "relative flex h-[360px] overflow-hidden rounded-xl border border-border/70 bg-background/70",
          glassBackground && "bg-background/80 backdrop-blur-xl",
        )}
        data-link-style={linkStyle}
        data-thread-mode={threadMode}
        data-testid="appearance-workspace-preview-surface"
      >
        <nav
          className={cn(
            "w-[132px] shrink-0 border-r border-border/55 bg-muted/30 p-2",
            glassBackground && "bg-muted/20",
          )}
        >
          <div className="mb-3 flex items-center gap-2 px-1.5 py-1">
            <div className="flex size-6 items-center justify-center rounded-lg bg-primary text-2xs font-bold text-primary-foreground">
              B
            </div>
            <span className="text-xs font-semibold">Buzz</span>
          </div>
          <div className="mb-3 flex items-center gap-1.5 rounded-lg border border-border/60 bg-background/70 px-2 py-1.5 text-2xs text-muted-foreground">
            <Search className="size-3" />
            Search
          </div>
          <button className={navItemClass(false)} type="button">
            <Inbox className="size-3.5" /> Inbox
          </button>
          <p className="mb-1 mt-4 px-2 text-3xs font-semibold uppercase tracking-wider text-muted-foreground/70">
            Channels
          </p>
          <button
            className={navItemClass(activeChannel === "design")}
            onClick={() => setActiveChannel("design")}
            type="button"
          >
            <Hash className="size-3.5" /> design
          </button>
          <button
            className={navItemClass(activeChannel === "general")}
            onClick={() => setActiveChannel("general")}
            type="button"
          >
            <Hash className="size-3.5" /> general
          </button>
        </nav>

        <div className="relative min-w-0 flex-1">
          <AnimatePresence custom={motionContext} initial={false} mode="sync">
            {channelPane}
          </AnimatePresence>
          <AnimatePresence custom={motionContext} initial={false} mode="sync">
            {threadOpen ? threadPanel : null}
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}

export function LinkPreviewStyleSetting({
  onPreviewChange,
}: {
  onPreviewChange?: (style: LinkPreviewStyle | null) => void;
}) {
  const style = useLinkPreviewStyle();
  const handlePreviewChange = React.useCallback(
    (nextStyle: LinkPreviewStyle | null) => {
      onPreviewChange?.(nextStyle);
    },
    [onPreviewChange],
  );
  return (
    <div
      data-testid="link-preview-style-group"
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          handlePreviewChange(null);
        }
      }}
      onFocusCapture={() => handlePreviewChange(style)}
      onPointerDownCapture={() => handlePreviewChange(style)}
    >
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">Link previews</p>
        </div>
        <SegmentedControl
          size="compact"
          legend="Link previews"
          onPreviewChange={handlePreviewChange}
          onValueChange={setLinkPreviewStyle}
          optionTestIdPrefix="link-preview-style"
          options={LINK_PREVIEW_STYLE_OPTIONS}
          testId="link-preview-style-control"
          value={style}
        />
      </SettingsOptionRow>
    </div>
  );
}

const THREAD_VIEW_MODE_OPTIONS: {
  value: ThreadViewMode;
  label: string;
  description: string;
}[] = [
  {
    value: "focus",
    label: "Focus",
    description: "Threads open over the channel",
  },
  {
    value: "split",
    label: "Split",
    description: "Threads open in a side panel next to the channel",
  },
];

/** Native window glass rows sit below theme and accent choices. */
export function GlassBackgroundSetting() {
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
        <p className="text-sm font-medium">Glass opacity</p>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
          id="glass-opacity-description"
        >
          Lower values reveal more of the desktop blur.
        </p>
      </div>
      <div className="flex w-64 shrink-0 items-center">
        <AvatarFramingSlider
          ariaDescribedBy="glass-opacity-description"
          ariaLabel="Glass opacity"
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
            Glass background
          </label>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {glassBackgroundSupported
              ? "Blur the desktop behind navigation while keeping content solid."
              : "Available in the macOS desktop app."}
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
export function ThreadLayoutSetting({
  onPreviewChange,
}: {
  onPreviewChange?: (mode: ThreadViewMode | null) => void;
}) {
  const threadViewMode = useThreadViewMode();
  const [previewMode, setPreviewMode] = React.useState<ThreadViewMode | null>(
    null,
  );
  const handlePreviewChange = React.useCallback(
    (nextMode: ThreadViewMode | null) => {
      setPreviewMode(nextMode);
      onPreviewChange?.(nextMode);
    },
    [onPreviewChange],
  );
  const { communities } = useCommunities();
  const showCommunityScope = communities.length > 1;
  const displayedMode = previewMode ?? threadViewMode;
  const activeOption =
    THREAD_VIEW_MODE_OPTIONS.find((option) => option.value === displayedMode) ??
    THREAD_VIEW_MODE_OPTIONS[0];

  return (
    <div
      data-testid="thread-layout-group"
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          handlePreviewChange(null);
        }
      }}
      onFocusCapture={() => handlePreviewChange(threadViewMode)}
      onPointerDownCapture={() => handlePreviewChange(threadViewMode)}
    >
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">
            Thread layout
            {showCommunityScope ? (
              <span className="font-normal text-muted-foreground">
                {" "}
                (all communities)
              </span>
            ) : null}
          </p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {activeOption.description}
          </p>
        </div>
        <SegmentedControl
          size="compact"
          legend="Thread layout"
          onPreviewChange={handlePreviewChange}
          onValueChange={setThreadViewMode}
          optionTestIdPrefix="thread-layout"
          options={THREAD_VIEW_MODE_OPTIONS}
          testId="thread-layout-control"
          value={threadViewMode}
        />
      </SettingsOptionRow>
    </div>
  );
}

/** Accent swatches — shared by the animated and reduced-motion reveal paths. */
export function AccentPickerContent({
  accentColor,
  setAccentColor,
}: {
  accentColor: string;
  setAccentColor: (value: string) => void;
}) {
  return (
    <SettingsOptionRow data-testid="accent-color-row">
      <div className="min-w-0">
        <p className="text-sm font-medium">Accent color</p>
      </div>
      <div
        className="min-w-0 max-w-[34rem] shrink-0 overflow-x-auto"
        data-testid="accent-color-options"
      >
        <div className="flex w-max min-w-full flex-nowrap justify-end gap-2">
          {ACCENT_COLORS.map((color) => {
            const isNeutral = color.value === NEUTRAL_ACCENT;
            const isSelected = accentColor === color.value;
            const swatchColor = isNeutral
              ? "hsl(var(--foreground))"
              : color.value;
            return (
              <button
                aria-label={`Use ${color.name} accent`}
                aria-pressed={isSelected}
                className="relative size-8 shrink-0 rounded-full border-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
                data-testid={`accent-color-${color.name.toLowerCase()}`}
                key={color.value}
                onClick={() => setAccentColor(color.value)}
                style={{ backgroundColor: swatchColor }}
                title={color.name}
                type="button"
              >
                <span
                  className="accent-color-selection-dot absolute left-1/2 top-1/2 size-2 rounded-full bg-white"
                  data-selected={isSelected}
                  data-testid={
                    isSelected ? "accent-color-selection" : undefined
                  }
                />
              </button>
            );
          })}
        </div>
      </div>
    </SettingsOptionRow>
  );
}
