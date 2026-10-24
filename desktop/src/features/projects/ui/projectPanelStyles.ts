/** Settings-aligned action button used at the right edge of panel headers. */
export const PROJECT_PANEL_ACTION_BUTTON_CLASS =
  "h-auto shrink-0 gap-1.5 rounded-full border-transparent bg-muted px-3 py-1.5 text-sm font-medium text-foreground shadow-none hover:bg-muted/80";

/**
 * Shared trigger for the selection dropdowns (repository, source, branch) so
 * they read as one consistent control family in the workspace header.
 */
export const PROJECT_PICKER_TRIGGER_CLASS =
  "h-7 min-w-0 max-w-full gap-1.5 rounded-md px-3 text-sm font-medium hover:border-input";

/** Bordered shell that lets the project page surface show through. */
export const PROJECT_DETAIL_PANEL_CLASS =
  "overflow-hidden rounded-xl border border-border/60 bg-transparent";

/** Centered, borderless reading column used by selected work-item details. */
export const PROJECT_DETAIL_READING_COLUMN_CLASS =
  "mx-auto w-full max-w-3xl overflow-hidden";

/** Shared translucent chrome for the paired project content/panel headers. */
export const PROJECT_COLUMN_HEADER_BACKDROP_CLASS =
  "bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55";

/** Rounded title row used above repository workspace lists and home sheets. */
export const PROJECT_SECTION_HEADER_CLASS = "mx-4 mb-2 rounded-md bg-muted/40";
