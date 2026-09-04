import type * as React from "react";
import type { ThreadPanelLayoutProps } from "@/features/channels/lib/threadPanelLayout";
import {
  THREAD_PANEL_COLUMN_CLASS,
  THREAD_PANEL_COMPOSER_GUTTER_CLASS,
  THREAD_PANEL_MESSAGE_GUTTER_CLASS,
} from "@/features/messages/lib/messageThreadPanelLayout";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { cn } from "@/shared/lib/cn";
import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanel";
import { Skeleton } from "@/shared/ui/skeleton";

/** Shared title row so the skeleton and loaded panel keep the same chrome. */
export function MessageThreadPanelHeader({
  headerLeading,
  headerTitle = "Thread",
  headerTitleAriaLabel,
  isFocusMode,
  isSinglePanelView,
  onClose,
  onHeaderTitleClick,
  showBackButton,
}: {
  headerLeading?: React.ReactNode;
  headerTitle?: string;
  headerTitleAriaLabel?: string;
  isFocusMode: boolean;
  isSinglePanelView: boolean;
  onClose: () => void;
  onHeaderTitleClick?: () => void;
  showBackButton?: boolean;
}) {
  const title = onHeaderTitleClick ? (
    <button
      aria-label={headerTitleAriaLabel ?? `Open ${headerTitle}`}
      className="min-w-0 max-w-full truncate text-left hover:underline"
      data-testid="message-thread-open-channel"
      onClick={onHeaderTitleClick}
      title={headerTitleAriaLabel ?? `Open ${headerTitle}`}
      type="button"
    >
      {headerTitle}
    </button>
  ) : (
    headerTitle
  );

  return (
    <AuxiliaryPanelHeader backdrop>
      <AuxiliaryPanelHeaderGroup
        backButtonAriaLabel="Back to conversation"
        backButtonTestId="message-thread-back"
        leading={headerLeading}
        // Focus drawers fill width via isSinglePanelView but use the scrim to go
        // back, so they omit this control. The narrow single-column view keeps it.
        onBack={
          (showBackButton ?? (isSinglePanelView && !isFocusMode))
            ? onClose
            : undefined
        }
      >
        <AuxiliaryPanelTitle>{title}</AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
    </AuxiliaryPanelHeader>
  );
}

type MessageThreadPanelSkeletonProps = ThreadPanelLayoutProps & {
  onClose: () => void;
  widthPx: number;
};

/** Placeholder row standing in for a thread message while replies load. */
export function ThreadMessageSkeleton({
  isHead = false,
}: {
  isHead?: boolean;
}) {
  return (
    <article className="relative flex items-start gap-2.5 rounded-2xl px-3 py-2">
      <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
      <div className="-mt-1 min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
          <Skeleton className="h-[15px] w-28" />
          <Skeleton className="h-3 w-16" />
        </div>
        <div className="mt-1 space-y-1.5 pb-2">
          <Skeleton className="h-4 w-full" />
          <Skeleton className={isHead ? "h-4 w-4/5" : "h-4 w-2/3"} />
        </div>
        <div className="flex items-center gap-4">
          <Skeleton className="h-4 w-8 rounded-full" />
          <Skeleton className="h-4 w-8 rounded-full" />
          <Skeleton className="h-4 w-8 rounded-full" />
        </div>
      </div>
    </article>
  );
}

function ThreadComposerSkeleton({
  columnMaxWidthPx,
}: {
  columnMaxWidthPx?: number;
}) {
  const hasConstrainedColumn = columnMaxWidthPx != null;

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
      <div
        className={cn(
          "pointer-events-auto",
          hasConstrainedColumn && THREAD_PANEL_COLUMN_CLASS,
        )}
        style={
          hasConstrainedColumn ? { maxWidth: columnMaxWidthPx } : undefined
        }
      >
        <div
          className={cn(
            "relative z-10 shrink-0 bg-transparent pb-2 pt-0",
            THREAD_PANEL_COMPOSER_GUTTER_CLASS,
          )}
        >
          <div className="relative isolate rounded-2xl border border-border/50 bg-background/80 px-3 pb-2 pt-3 shadow-none backdrop-blur-md sm:px-4">
            <Skeleton className="h-5 w-48 max-w-full" />
            <div className="mt-4 flex items-center gap-2">
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="ml-auto h-8 w-20 rounded-full" />
            </div>
          </div>
        </div>
        <div
          className={cn(
            "h-7 bg-background pb-1 pt-0",
            THREAD_PANEL_COMPOSER_GUTTER_CLASS,
          )}
        />
      </div>
    </div>
  );
}

/** Loading state for the thread panel, in every layout the real panel supports. */
export function MessageThreadPanelSkeleton({
  canResetWidth,
  columnMaxWidthPx,
  enterMotion,
  headerLeading,
  headerTitle,
  headerTitleAriaLabel,
  isFocusMode,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  onHeaderTitleClick,
  onResetWidth,
  onResizeStart,
  showBackButton,
  splitPaneClamp,
  testId = "message-thread-panel",
  widthPx,
  transparentChrome = false,
}: MessageThreadPanelSkeletonProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const hasConstrainedColumn = columnMaxWidthPx != null;
  useEscapeKey(onClose, isOverlay || isSinglePanelView || isFocusMode);

  const threadBody = (
    <AuxiliaryPanelBody
      className="overflow-y-auto overflow-x-hidden overscroll-contain pb-24"
      data-testid="message-thread-loading"
    >
      <div
        className={cn(hasConstrainedColumn && THREAD_PANEL_COLUMN_CLASS)}
        style={
          hasConstrainedColumn ? { maxWidth: columnMaxWidthPx } : undefined
        }
      >
        <div
          className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-1 pt-0")}
          data-testid="message-thread-head-loading"
        >
          <ThreadMessageSkeleton isHead />
        </div>
        <div
          className={cn(
            "space-y-2.5 pb-3 pt-1",
            THREAD_PANEL_MESSAGE_GUTTER_CLASS,
          )}
        >
          <ThreadMessageSkeleton />
          <ThreadMessageSkeleton />
          <div className="ml-[58px] flex items-center gap-1.5 pt-0.5">
            <Skeleton className="h-7 w-7 rounded-full" />
            <Skeleton className="h-7 w-7 rounded-full" />
            <Skeleton className="h-4 w-28 rounded-full" />
          </div>
        </div>
      </div>
    </AuxiliaryPanelBody>
  );

  return (
    <AuxiliaryPanel
      canResetWidth={canResetWidth}
      className="relative"
      enterMotion={enterMotion ?? !isFocusMode}
      footer={<ThreadComposerSkeleton columnMaxWidthPx={columnMaxWidthPx} />}
      header={
        <MessageThreadPanelHeader
          headerLeading={headerLeading}
          headerTitle={headerTitle}
          headerTitleAriaLabel={headerTitleAriaLabel}
          isFocusMode={isFocusMode}
          isSinglePanelView={isSinglePanelView}
          onClose={onClose}
          onHeaderTitleClick={onHeaderTitleClick}
          showBackButton={showBackButton}
        />
      }
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      splitPaneClamp={splitPaneClamp}
      testId={testId}
      transparentChrome={transparentChrome}
      widthPx={widthPx}
    >
      {threadBody}
    </AuxiliaryPanel>
  );
}
