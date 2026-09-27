import { BookOpen } from "lucide-react";
import * as React from "react";

import { ExportBrowserShareButton } from "@/features/browser-share/ui/ExportBrowserShareButton";
import type { BrowserShareSource } from "@/features/browser-share/lib/types";
import { formatSidebarMenuCount } from "@/features/sidebar/lib/sidebarMenuCounts";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

import { useRunbookPendingCount, useSiteRunbook } from "../hooks";
import type { SiteRunbookRef } from "../lib/types";
import { SiteRunbookPanel } from "./SiteRunbookPanel";

export type SiteRunbookExportShare = {
  url: string;
  title?: string;
  source: BrowserShareSource;
};

export function SiteRunbookDialog({
  open,
  onOpenChange,
  runbookRef,
  title = "How to use this site",
  exportShare,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  runbookRef: SiteRunbookRef;
  title?: string;
  /** When set, footer includes Export (URL + runbook JSON). */
  exportShare?: SiteRunbookExportShare;
}) {
  const api = useSiteRunbook(open ? runbookRef : null);
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        footer={
          <div className="flex items-center justify-between gap-2">
            {exportShare ? (
              <ExportBrowserShareButton
                runbookRef={runbookRef}
                source={exportShare.source}
                testId="site-runbook-export"
                title={exportShare.title}
                url={exportShare.url}
                variant="ghost"
              />
            ) : (
              <span />
            )}
            <Button
              onClick={() => onOpenChange(false)}
              type="button"
              variant="secondary"
            >
              Done
            </Button>
          </div>
        }
        title={title}
      >
        <SiteRunbookPanel
          onAccept={api.accept}
          onAddManual={api.addManual}
          onArchive={api.archive}
          onDelete={api.remove}
          onReject={api.reject}
          onSetBrief={api.setBrief}
          onSetPersisted={api.setPersisted}
          onUpdate={api.update}
          runbook={api.runbook}
        />
      </ChooserDialogContent>
    </Dialog>
  );
}

export function SiteRunbookOpenButton({
  runbookRef,
  label = "Runbook",
  testId,
  exportShare,
  pendingCount: pendingCountProp,
}: {
  runbookRef: SiteRunbookRef;
  label?: string;
  testId?: string;
  exportShare?: SiteRunbookExportShare;
  /** Optional override when the parent already subscribed. */
  pendingCount?: number;
}) {
  const [open, setOpen] = React.useState(false);
  const hookedPending = useRunbookPendingCount(
    pendingCountProp === undefined ? runbookRef : null,
  );
  const pendingCount = pendingCountProp ?? hookedPending;
  const pendingDisplay =
    pendingCount > 0 ? formatSidebarMenuCount(pendingCount) : 0;
  const pendingTestId = testId
    ? `${testId}-pending-count`
    : "site-runbook-pending-count";

  return (
    <>
      <Button
        data-testid={testId ?? "site-runbook-open"}
        onClick={() => setOpen(true)}
        size="xs"
        type="button"
        variant="ghost"
      >
        <BookOpen className="mr-1 h-3 w-3" />
        {label}
        {pendingDisplay > 0 ? (
          <Badge
            className="ml-1 normal-case tracking-normal"
            data-testid={pendingTestId}
            variant="warning"
          >
            {pendingDisplay}
          </Badge>
        ) : null}
      </Button>
      <SiteRunbookDialog
        exportShare={exportShare}
        onOpenChange={setOpen}
        open={open}
        runbookRef={runbookRef}
      />
    </>
  );
}
