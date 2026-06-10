/** Fixed global top chrome height (search bar / drag region). */
export const SPROUT_TOP_CHROME_HEIGHT_VAR = "--sprout-top-chrome-height";

/** Scroll-content inset for channel layouts with an overlaid channel header. */
export const SPROUT_CHANNEL_CONTENT_TOP_PADDING_VAR =
  "--sprout-channel-content-top-padding";

export const TOP_CHROME_HEIGHT_DEFAULT = "2.5rem";
export const CHANNEL_CONTENT_TOP_PADDING_DEFAULT = "5.75rem";

/** No-flash defaults applied to the app `<main>` inset. */
export const MAIN_INSET_CHROME_VARS_CLASS =
  "[--sprout-top-chrome-height:2.5rem] [--sprout-channel-content-top-padding:5.75rem]";

/** Tailwind class fragments for layout under the global top chrome. */
export const topChromeInset = {
  /** Absolute/fixed top offset below the search bar. */
  top: "top-(--sprout-top-chrome-height,2.5rem)",
  /** Padding-top clearing the global top chrome. */
  padding: "pt-(--sprout-top-chrome-height,2.5rem)",
  /** `after:` pseudo-element top offset. */
  afterTop: "after:top-(--sprout-top-chrome-height,2.5rem)",
  /** Horizontal divider at the bottom edge of the global top chrome inset. */
  divider:
    "before:pointer-events-none before:absolute before:inset-x-0 before:top-(--sprout-top-chrome-height,2.5rem) before:h-px before:bg-border/35 before:content-['']",
  /** Shared header backdrop and bottom border below the inset row. */
  headerBase:
    "relative z-40 shrink-0 border-b border-border/35 bg-background/75 backdrop-blur-md supports-backdrop-filter:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/35",
  /** Vertical pane divider starting below the global top chrome. */
  verticalDivider:
    "after:pointer-events-none after:absolute after:bottom-0 after:right-0 after:top-(--sprout-top-chrome-height,2.5rem) after:z-40 after:w-px after:bg-border/35 after:content-['']",
} as const;

/** Tailwind class fragments for the global top chrome backdrop strip. */
export const topChromeBackdrop = {
  /** Height matching the global top chrome search/drag strip. */
  height: "h-(--sprout-top-chrome-height,2.5rem)",
  /** `after:` pseudo-element offset aligned to the bottom of top chrome. */
  dividerTop: "after:top-(--sprout-top-chrome-height,2.5rem)",
} as const;

/** Tailwind class fragments for measured channel header chrome. */
export const channelChrome = {
  /** Padding-top that clears the measured channel header chrome. */
  contentPadding: "pt-(--sprout-channel-content-top-padding,5.75rem)",
  /** Height matching the measured channel header chrome. */
  headerHeight: "h-(--sprout-channel-content-top-padding,5.75rem)",
  /** Negative margin for overlaid channel chrome that should not affect flow. */
  negativeMargin: "-mb-(--sprout-channel-content-top-padding,5.75rem)",
  /** Visual chrome for channel header icon actions (pair with `size="icon"`). */
  headerIconButton:
    "rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground",
  /** Channel header members button with icon + count label. */
  headerMembersButton:
    "h-8 gap-1.5 rounded-lg border border-border/40 px-2.5 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-4.5",
  /** Icon button sizing for right auxiliary panel header close actions. */
  splitPanelCloseButton:
    "rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground",
  /** Title typography for right auxiliary panel headers. */
  splitPanelTitle:
    "translate-y-px text-base font-semibold leading-6 tracking-tight",
} as const;
