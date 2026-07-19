/** Inbox-aligned spacing and typography for top-level Projects list rows. */
export const PROJECT_LIST_CONTAINER_CLASS =
  "divide-y divide-border/60 overflow-hidden rounded-xl border border-border/60 bg-card";

export const PROJECT_LIST_ROW_CLASS =
  "group relative px-3 py-3 transition-colors duration-150 hover:bg-muted/20";

export const PROJECT_LIST_ROW_CONTENT_CLASS =
  "flex min-w-0 items-start gap-2.5";

export const PROJECT_LIST_ROW_TITLE_CLASS =
  "truncate text-sm font-semibold leading-5 text-foreground";

export const PROJECT_LIST_ROW_META_TEXT_CLASS =
  "text-2xs leading-3 text-muted-foreground";

export const PROJECT_LIST_ROW_META_CLASS = `flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 ${PROJECT_LIST_ROW_META_TEXT_CLASS}`;

export const PROJECT_LIST_ROW_SUBTEXT_CLASS =
  "mt-0.5 text-sm leading-5 text-muted-foreground";

export const PROJECT_LIST_ROW_PREVIEW_CLASS = `line-clamp-1 ${PROJECT_LIST_ROW_SUBTEXT_CLASS}`;

export const PROJECT_LIST_ROW_TRAILING_CLASS =
  "relative z-10 ml-auto flex shrink-0 items-center gap-3";

export const PROJECT_LIST_ROW_STATUS_CLASS =
  "hidden w-20 shrink-0 text-left text-2xs leading-3 text-muted-foreground md:block";

export const PROJECT_LIST_ROW_DATE_CLASS =
  "hidden w-24 shrink-0 text-right text-xs leading-4 text-muted-foreground/70 sm:block";
