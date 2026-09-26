import { BookOpen } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

import { useSiteRunbook } from "../hooks";
import type { SiteRunbookRef } from "../lib/types";
import { SiteRunbookPanel } from "./SiteRunbookPanel";

export function SiteRunbookDialog({
  open,
  onOpenChange,
  runbookRef,
  title = "How to use this site",
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  runbookRef: SiteRunbookRef;
  title?: string;
}) {
  const api = useSiteRunbook(open ? runbookRef : null);
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        footer={
          <div className="flex justify-end">
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
}: {
  runbookRef: SiteRunbookRef;
  label?: string;
  testId?: string;
}) {
  const [open, setOpen] = React.useState(false);
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
      </Button>
      <SiteRunbookDialog
        onOpenChange={setOpen}
        open={open}
        runbookRef={runbookRef}
      />
    </>
  );
}
