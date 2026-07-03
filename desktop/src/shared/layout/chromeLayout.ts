// Fixed px on purpose: the top chrome strip hosts the fixed-size macOS
// traffic lights and the px-pinned nav buttons (see `AppTopChrome`), so its
// height must not follow the Cmd +/- rem text scale either. Deliberate
// exception to the rem-first rule. 40px == the old 2.5rem at default zoom.
export const TOP_CHROME_HEIGHT_DEFAULT = "40px";
export const CHANNEL_CONTENT_TOP_PADDING_DEFAULT = "5.75rem";

export const chromeCssVars = {
  topChromeHeight: "--buzz-top-chrome-height",
  channelContentTopPadding: "--buzz-channel-content-top-padding",
} as const;

export const chromeCssVarDefaults = {
  [chromeCssVars.topChromeHeight]: TOP_CHROME_HEIGHT_DEFAULT,
  [chromeCssVars.channelContentTopPadding]: CHANNEL_CONTENT_TOP_PADDING_DEFAULT,
} as const;

export const channelContentTopPaddingMeasurement = {
  cssVariable: chromeCssVars.channelContentTopPadding,
  resetValue: chromeCssVarDefaults[chromeCssVars.channelContentTopPadding],
} as const;

/** Tailwind class fragments for content below the in-flow global top chrome. */
export const topChromeInset = {
  /** Absolute/fixed top offset inside the content row. */
  top: "top-0",
  /** Content now sits below the global chrome in normal layout flow. */
  padding: "pt-0",
  /** `after:` pseudo-element top offset. */
  afterTop: "after:top-0",
  /** Horizontal divider at the top edge of the content row. */
  divider:
    "before:pointer-events-none before:absolute before:inset-x-0 before:top-0 before:h-px before:bg-border/35 before:content-['']",
  /** Shared header backdrop and bottom border below the inset row. */
  headerBase:
    "relative z-40 shrink-0 bg-background/75 backdrop-blur-md supports-backdrop-filter:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/35",
  /** Vertical pane divider starting at the top of the content row. */
  verticalDivider:
    "after:pointer-events-none after:absolute after:bottom-0 after:right-0 after:top-0 after:z-40 after:w-px after:bg-border/35 after:content-['']",
} as const;

/** Tailwind class fragments for the global top chrome backdrop strip. */
export const topChromeBackdrop = {
  /** Height matching the global top chrome search/drag strip. */
  height: "h-(--buzz-top-chrome-height,40px)",
  /** `after:` pseudo-element offset aligned to the bottom of top chrome. */
  dividerTop: "after:top-(--buzz-top-chrome-height,40px)",
} as const;

/** Tailwind class fragments for measured channel header chrome. */
export const channelChrome = {
  /** Padding-top that clears the measured channel header chrome. */
  contentPadding: "pt-(--buzz-channel-content-top-padding,5.75rem)",
  /** Absolute/fixed top offset below the measured channel header chrome. */
  top: "top-(--buzz-channel-content-top-padding,5.75rem)",
  /** Height matching the measured channel header chrome. */
  headerHeight: "h-(--buzz-channel-content-top-padding,5.75rem)",
  /** Negative margin for overlaid channel chrome that should not affect flow. */
  negativeMargin: "-mb-(--buzz-channel-content-top-padding,5.75rem)",
} as const;
