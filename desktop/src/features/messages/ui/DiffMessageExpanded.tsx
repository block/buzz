import { Rows3, SplitSquareVertical } from "lucide-react";
import { useMemo, useState } from "react";

import { getDiffTitleBadge } from "@/features/messages/lib/parseDiff";
import { DiffViewer } from "@/features/messages/ui/DiffViewer";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type DiffMessageExpandedProps = {
  content: string;
  filePath?: string;
  onClose: () => void;
};

export default function DiffMessageExpanded({
  content,
  filePath,
  onClose,
}: DiffMessageExpandedProps) {
  const [viewType, setViewType] = useState<"split" | "unified">("unified");

  const titleBadge = useMemo(
    () => getDiffTitleBadge(content, filePath),
    [content, filePath],
  );

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      open
    >
      <DialogContent className="max-w-5xl w-full h-[80vh] flex flex-col p-0 gap-0">
        <DialogHeader className="shrink-0 border-b border-border/50 px-4 py-3 pr-14">
          <div className="flex flex-wrap items-center gap-2">
            <DialogTitle className="min-w-0 truncate font-mono text-sm font-medium">
              {filePath ?? "Diff Viewer"}
            </DialogTitle>
            {titleBadge && (
              <span className="shrink-0 rounded-md border border-border/60 px-1.5 py-0.5 text-2xs uppercase tracking-[0.14em] text-muted-foreground">
                {titleBadge}
              </span>
            )}
            <div className="ml-auto flex items-center gap-1 rounded-lg border border-border/60 bg-muted/30 p-1">
              <Button
                className="h-7 px-2"
                onClick={() => {
                  setViewType("unified");
                }}
                size="sm"
                type="button"
                variant={viewType === "unified" ? "secondary" : "ghost"}
              >
                <Rows3 className="h-4 w-4" />
                Unified
              </Button>
              <Button
                className="h-7 px-2"
                onClick={() => {
                  setViewType("split");
                }}
                size="sm"
                type="button"
                variant={viewType === "split" ? "secondary" : "ghost"}
              >
                <SplitSquareVertical className="h-4 w-4" />
                Split
              </Button>
            </div>
          </div>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
          <DiffViewer
            content={content}
            fallbackFilePath={filePath}
            viewType={viewType}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
