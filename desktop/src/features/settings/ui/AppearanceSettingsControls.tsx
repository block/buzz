import * as React from "react";
import type { ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Eye } from "lucide-react";
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

function ConversationPreview() {
  return (
    <div className="px-4 py-3" data-testid="conversation-preview">
      <div
        aria-hidden="true"
        className="relative overflow-hidden rounded-xl border border-border/65 bg-transparent"
        data-testid="conversation-preview-surface"
      >
        <span className="absolute right-3.5 top-3 inline-flex items-center gap-1 text-2xs font-medium text-muted-foreground/55">
          <Eye aria-hidden="true" className="size-3" />
          Preview
        </span>
        <div className="p-4" data-testid="conversation-preview-content">
          <ConversationDensityPreviewMessage
            avatar="M"
            author="Maya"
            timestamp="9:41"
          >
            The revised conversation layout is ready to review.
          </ConversationDensityPreviewMessage>
          <ConversationDensityPreviewMessage
            avatar="T"
            author="Theo"
            timestamp="9:43"
          >
            <p>
              I added a longer message so you can compare line height and text
              spacing.
            </p>
            <p className="mt-conversation-paragraph">
              The same rhythm carries through channels, threads, DMs, and Inbox.
            </p>
          </ConversationDensityPreviewMessage>
        </div>
      </div>
    </div>
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
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            Applies across conversations and interface text
          </p>
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
      <SettingsOptionRow data-testid="conversation-density-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">Conversation density</p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            Spacing in conversations and Markdown content across Buzz
          </p>
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
      <ConversationPreview />
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

function LinkPreviewSample({ style }: { style: LinkPreviewStyle }) {
  const { isDark } = useTheme();
  const preview = React.useMemo<ResolvedLinkPreview>(
    () => ({
      ...LINK_PREVIEW_SAMPLE_BASE,
      imageDataUrl: buzzGradientSampleImage(isDark),
    }),
    [isDark],
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
          Preview
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
  const style = useLinkPreviewStyle();
  const [previewStyle, setPreviewStyle] =
    React.useState<LinkPreviewStyle | null>(null);
  const displayedStyle = previewStyle ?? style;
  const activeOption =
    LINK_PREVIEW_STYLE_OPTIONS.find(
      (option) => option.value === displayedStyle,
    ) ?? LINK_PREVIEW_STYLE_OPTIONS[0];

  return (
    <div data-testid="link-preview-style-group">
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">Link previews</p>
          <p
            className="text-sm font-normal text-muted-foreground/70"
            data-settings-subcopy
          >
            {activeOption.description}
          </p>
        </div>
        <SegmentedControl
          size="compact"
          legend="Link previews"
          onPreviewChange={setPreviewStyle}
          onValueChange={setLinkPreviewStyle}
          optionTestIdPrefix="link-preview-style"
          options={LINK_PREVIEW_STYLE_OPTIONS}
          testId="link-preview-style-control"
          value={style}
        />
      </SettingsOptionRow>
      <LinkPreviewSample style={displayedStyle} />
    </div>
  );
}

export function AccentPickerContent({
  accentColor,
  isDark,
  setAccentColor,
}: {
  accentColor: string;
  isDark: boolean;
  setAccentColor: (value: string) => void;
}) {
  return (
    <SettingsOptionRow className="items-start">
      <div className="min-w-0">
        <p className="text-sm font-medium">Accent color</p>
        <p
          className="text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          Choose the highlight color used throughout Buzz.
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
                aria-label={`Use ${color.name} accent`}
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
