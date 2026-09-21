import { X } from "lucide-react";

import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Skeleton } from "@/shared/ui/skeleton";

type WorkflowUnavailableDialogProps = {
  loading: boolean;
  onOpenChange: (open: boolean) => void;
  onRetry: () => void;
  open: boolean;
};

export function WorkflowUnavailableDialog({
  loading,
  onOpenChange,
  onRetry,
  open,
}: WorkflowUnavailableDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-lg" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>
            {loading
              ? t("workflows.unavailable.loading")
              : t("workflows.unavailable.title")}
          </DialogTitle>
          <DialogDescription>
            {loading
              ? t("workflows.unavailable.loading-description")
              : t("workflows.unavailable.description")}
          </DialogDescription>
        </DialogHeader>
        {loading ? (
          <div
            aria-label={t("workflows.unavailable.loading-aria")}
            className="space-y-3"
            role="status"
          >
            <Skeleton className="h-5 w-48" />
            <Skeleton className="h-24 w-full rounded-xl" />
          </div>
        ) : (
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                <X className="h-4 w-4" />
                {t("shared.ui.close")}
              </Button>
            </DialogClose>
            <Button onClick={onRetry} type="button">
              {t("workflows.unavailable.retry")}
            </Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}
