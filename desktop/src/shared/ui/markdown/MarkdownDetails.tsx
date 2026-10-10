import { ChevronRight } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

import { useMarkdownRuntime } from "./runtimeContext";

/**
 * Open sections, keyed by message id and the section key from
 * `remarkDetails`. Module-level so a reader's choice survives the timeline's
 * per-channel remount and in-place edits of the message (status boards);
 * cleared on app restart and community switch, collapsed being the default.
 * Insertion-ordered and capped, so the oldest choices are forgotten first.
 */
const OPEN_SECTIONS_LIMIT = 500;
const openSections = new Set<string>();

/** Community switches swap message spaces (see `resetCommunityState`). */
export function clearOpenMarkdownSections() {
  openSections.clear();
}

function rememberOpen(storeKey: string, open: boolean) {
  openSections.delete(storeKey);
  if (!open) return;
  openSections.add(storeKey);
  if (openSections.size > OPEN_SECTIONS_LIMIT) {
    const oldest = openSections.values().next().value;
    if (oldest !== undefined) openSections.delete(oldest);
  }
}

const BODY_CLASS =
  "[&>*+*]:mt-3 [&>ol]:space-y-conversation-list [&>ul]:space-y-conversation-list";

export function MarkdownDetails({
  children,
  detailsKey,
  interactive,
}: {
  children?: React.ReactNode;
  detailsKey?: string;
  interactive: boolean;
}) {
  const { messageId } = useMarkdownRuntime();
  const [summary, ...body] = React.Children.toArray(children).filter(
    (child) => !(typeof child === "string" && child.trim() === ""),
  );

  // Compact previews (and other read-only surfaces) show the section inline,
  // like mobile previews do; the summary renders as plain emphasis.
  if (!interactive) {
    return (
      <div className={BODY_CLASS} data-details="">
        {summary}
        {body}
      </div>
    );
  }

  const storeKey =
    messageId && detailsKey ? `${messageId}\u0000${detailsKey}` : null;
  // Keyed so an edit that renames or renumbers the section starts from that
  // section's own remembered state, not the previous occupant's.
  return (
    <DetailsDisclosure
      body={body}
      key={storeKey ?? detailsKey}
      storeKey={storeKey}
      summary={summary}
    />
  );
}

type DetailsToggle = { open: boolean; toggle: () => void };

/** Lets the summary, possibly nested in a heading, own the toggle button. */
const DetailsToggleContext = React.createContext<DetailsToggle | null>(null);

function DetailsDisclosure({
  body,
  storeKey,
  summary,
}: {
  body: React.ReactNode[];
  storeKey: string | null;
  summary: React.ReactNode;
}) {
  const [open, setOpen] = React.useState(
    () => storeKey !== null && openSections.has(storeKey),
  );

  const toggle = React.useMemo<DetailsToggle>(
    () => ({
      open,
      toggle: () => {
        if (storeKey !== null) rememberOpen(storeKey, !open);
        setOpen(!open);
      },
    }),
    [open, storeKey],
  );

  return (
    <div data-details="" data-open={open ? "true" : "false"}>
      <DetailsToggleContext.Provider value={toggle}>
        {summary}
      </DetailsToggleContext.Provider>
      {open ? (
        <div className={cn("mt-1.5 pl-5", BODY_CLASS)}>{body}</div>
      ) : null}
    </div>
  );
}

/** The section title (inline formatting only, see `remarkDetails`): the
 * disclosure button, or plain emphasis on read-only surfaces. */
export function MarkdownDetailsSummary({
  children,
  ...props
}: {
  children?: React.ReactNode;
  "data-heading"?: string;
}) {
  const details = React.useContext(DetailsToggleContext);
  // Heading titles keep the heading's own weight.
  const weight = props["data-heading"] != null ? "" : "font-medium";
  if (!details) return <span className={weight}>{children}</span>;
  return (
    <button
      aria-expanded={details.open}
      className="inline-flex cursor-pointer items-center gap-1 text-left hover:underline"
      onClick={details.toggle}
      type="button"
    >
      <ChevronRight
        aria-hidden="true"
        className={cn(
          "size-4 shrink-0 transition-transform motion-reduce:transition-none",
          details.open && "rotate-90",
        )}
      />
      <span className={weight}>{children}</span>
    </button>
  );
}
