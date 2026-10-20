import * as React from "react";
import { ChevronDown, ChevronUp, ListChecks, X } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { AuxiliaryPanelContext } from "@/shared/layout/auxiliaryPanelContext";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import {
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import { Skeleton } from "@/shared/ui/skeleton";
import { Spinner } from "@/shared/ui/spinner";
import { isCatchUpConfigured } from "./threadCatchUp";
import { useThreadCatchUp } from "./ThreadCatchUpContext";
import { ThreadCatchUpRangeChooser } from "./ThreadCatchUpRangeChooser";
import { ThreadCatchUpRecapView } from "./ThreadCatchUpRecapView";
import { ThreadCatchUpSetup } from "./ThreadCatchUpSetup";
import { ThreadCatchUpShared } from "./ThreadCatchUpShared";

const PANE_ID = "thread-catch-up-pane";
const TOGGLE_ID = "thread-catch-up-toggle";
const COMPACT_HEADER_WIDTH_PX = 400;

/** Header toggle for the recap pane; renders only for eligible threads. */
export function ThreadCatchUpHeaderAction({
  threadHeadId,
}: {
  threadHeadId: string | null | undefined;
}) {
  const controller = useThreadCatchUp(threadHeadId);
  const panel = React.useContext(AuxiliaryPanelContext);
  if (!controller) return null;
  const compact = (panel?.widthPx ?? 0) < COMPACT_HEADER_WIDTH_PX;
  const label = controller.isOpen ? "Hide catch-up" : "Catch me up";
  return (
    <Button
      aria-controls={controller.isOpen ? PANE_ID : undefined}
      aria-expanded={controller.isOpen}
      aria-label={compact ? label : undefined}
      className={cn("shrink-0", controller.isOpen && "bg-accent")}
      data-testid="thread-catch-up-toggle"
      id={TOGGLE_ID}
      onClick={() => controller.setIsOpen(!controller.isOpen)}
      size={compact ? "icon" : "sm"}
      title={compact ? label : undefined}
      type="button"
      variant="ghost"
    >
      <ListChecks aria-hidden />
      {compact ? null : <span>Catch me up</span>}
    </Button>
  );
}

/**
 * The recap pane, laid over the top of the thread so the conversation's
 * scroll position and composer are untouched. Collapsing keeps every draft
 * mounted; closing returns focus to the header toggle.
 */
export function ThreadCatchUpPane({
  columnMaxWidthPx,
  currentPubkey,
  profiles,
  threadHeadId,
}: {
  columnMaxWidthPx?: number;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
  threadHeadId: string | null | undefined;
}) {
  const controller = useThreadCatchUp(threadHeadId);
  const panel = React.useContext(AuxiliaryPanelContext);
  const [expanded, setExpanded] = React.useState(true);
  const [editingSettings, setEditingSettings] = React.useState(false);
  const paneRef = React.useRef<HTMLElement>(null);
  const isOpen = controller?.isOpen ?? false;

  React.useEffect(() => {
    if (isOpen) {
      setExpanded(true);
      paneRef.current?.focus({ preventScroll: true });
    }
  }, [isOpen]);

  if (!controller?.isOpen || !currentPubkey) return null;
  const { phase, settingsQuery } = controller;
  const settings = settingsQuery.data;
  const configured = isCatchUpConfigured(settings);
  const model = settings?.model.trim() ?? "";
  const close = () => {
    controller.setIsOpen(false);
    document.getElementById(TOGGLE_ID)?.focus();
  };

  let status: string | null = null;
  if (phase.kind === "generating") status = "Generating…";
  else if (phase.kind === "ready") {
    status = `${phase.recap.points.length} point${phase.recap.points.length === 1 ? "" : "s"}`;
  } else if (phase.kind === "error") status = "Failed";

  let body: React.ReactNode;
  if (settingsQuery.isPending) {
    body = (
      <div
        aria-label="Loading catch-up setup"
        className="space-y-2"
        role="status"
      >
        <Skeleton className="h-4 w-3/4" />
        <Skeleton className="h-8 w-full" />
      </div>
    );
  } else if (settingsQuery.isError) {
    body = (
      <Alert variant="destructive">
        <AlertDescription>
          Catch-up setup could not be loaded. {settingsQuery.error.message}
        </AlertDescription>
        <Button
          className="mt-2"
          onClick={() => void settingsQuery.refetch()}
          size="xs"
          type="button"
          variant="outline"
        >
          Try again
        </Button>
      </Alert>
    );
  } else if (!configured || editingSettings) {
    body = (
      <ThreadCatchUpSetup
        initial={settings}
        onCancel={configured ? () => setEditingSettings(false) : undefined}
        onSaved={() => setEditingSettings(false)}
        saveSettings={controller.saveSettings}
      />
    );
  } else if (phase.kind === "generating") {
    body = (
      <div
        aria-live="polite"
        className="flex flex-wrap items-center gap-3 py-1 text-sm text-muted-foreground"
        data-testid="thread-catch-up-generating"
      >
        <Spinner aria-hidden className="size-4" />
        <span className="min-w-0 flex-1">
          Reading the thread with {model} on this computer…
        </span>
        <Button
          data-testid="thread-catch-up-cancel"
          onClick={controller.cancel}
          size="sm"
          type="button"
          variant="outline"
        >
          Cancel
        </Button>
      </div>
    );
  } else if (phase.kind === "error") {
    body = (
      <Alert data-testid="thread-catch-up-error" variant="destructive">
        <AlertDescription>
          <span className="font-medium">The recap could not be generated.</span>{" "}
          {phase.message}
        </AlertDescription>
        <div className="mt-2 flex flex-wrap gap-2">
          <Button
            data-testid="thread-catch-up-retry"
            onClick={() => controller.generate(phase.choice)}
            size="xs"
            type="button"
            variant="outline"
          >
            Retry
          </Button>
          <Button
            onClick={controller.cancel}
            size="xs"
            type="button"
            variant="ghost"
          >
            Choose another period
          </Button>
        </div>
      </Alert>
    );
  } else if (phase.kind === "ready") {
    body = (
      <ThreadCatchUpRecapView
        controller={controller}
        currentPubkey={currentPubkey}
        phase={phase}
      />
    );
  } else {
    body = (
      <ThreadCatchUpRangeChooser
        baseline={controller.baseline}
        model={model}
        onChangeSetup={() => setEditingSettings(true)}
        onGenerate={controller.generate}
      />
    );
  }

  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 z-30 px-3 pt-1",
        panel?.mode === "docked" ? channelChrome.top : "top-13",
      )}
      data-testid="thread-catch-up-layer"
    >
      <section
        aria-labelledby="thread-catch-up-title"
        className={cn(
          "pointer-events-auto mx-auto flex w-full flex-col overflow-hidden rounded-2xl outline-hidden animate-in fade-in-0 duration-150 motion-reduce:animate-none",
          POPOVER_SURFACE_CLASS,
        )}
        data-testid="thread-catch-up-pane"
        id={PANE_ID}
        ref={paneRef}
        style={{
          ...POPOVER_SHADOW_STYLE,
          ...(columnMaxWidthPx ? { maxWidth: columnMaxWidthPx } : {}),
        }}
        tabIndex={-1}
      >
        <div className="flex min-h-10 items-center gap-2 py-1 pl-3.5 pr-1.5">
          <h3
            className="min-w-0 truncate text-sm font-medium"
            id="thread-catch-up-title"
          >
            Catch me up
          </h3>
          {status ? (
            <span className="shrink-0 text-xs text-muted-foreground">
              {status}
            </span>
          ) : null}
          <div className="ml-auto flex shrink-0 items-center gap-0.5">
            <Button
              aria-expanded={expanded}
              aria-label={expanded ? "Collapse catch-up" : "Expand catch-up"}
              data-testid="thread-catch-up-collapse"
              onClick={() => setExpanded((value) => !value)}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              {expanded ? (
                <ChevronUp aria-hidden />
              ) : (
                <ChevronDown aria-hidden />
              )}
            </Button>
            <Button
              aria-label="Close catch-up"
              data-testid="thread-catch-up-close"
              onClick={close}
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <X aria-hidden />
            </Button>
          </div>
        </div>
        <div
          className="max-h-[min(36rem,calc(100dvh-18rem))] space-y-4 overflow-y-auto overscroll-contain border-t border-border/40 px-3.5 pb-3.5 pt-3"
          data-testid="thread-catch-up-body"
          hidden={!expanded}
        >
          {body}
          <ThreadCatchUpShared
            controller={controller}
            currentPubkey={currentPubkey}
            profiles={profiles}
          />
        </div>
      </section>
    </div>
  );
}
