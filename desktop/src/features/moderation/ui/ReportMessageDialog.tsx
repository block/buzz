import { Flag } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useSubmitReportMutation } from "@/features/moderation/hooks";
import type { ReportType } from "@/features/moderation/hooks";
import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";

export function ReportMessageDialog({
  open,
  onOpenChange,
  authorPubkey,
  eventId,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Display author of the reported message (the `p` tag target). */
  authorPubkey: string;
  /** Reported message event id (the `e` tag target). */
  eventId: string;
}) {
  const { t } = useTranslation();
  const submitReport = useSubmitReportMutation();
  const [category, setCategory] = React.useState<ReportType | null>(null);
  const [note, setNote] = React.useState("");

  /** NIP-56 report categories, in the order shown to the reporter. `other` is
   *  last so it reads as the fallback rather than a first-class choice. The
   *  `value` enums are protocol data and stay untranslated. */
  const REPORT_CATEGORIES: { value: ReportType; label: string }[] = [
    { value: "spam", label: t("moderation.report.spam") },
    {
      value: "profanity",
      label: t("moderation.report.profanity-or-hate-speech"),
    },
    {
      value: "nudity",
      label: t("moderation.report.nudity-or-sexual-content"),
    },
    { value: "impersonation", label: t("moderation.report.impersonation") },
    { value: "malware", label: t("moderation.report.malware-or-scam") },
    { value: "illegal", label: t("moderation.report.illegal-content") },
    { value: "other", label: t("moderation.report.other") },
  ];

  // Reset the form each time the dialog opens so a prior report's selection
  // never leaks into the next one.
  React.useEffect(() => {
    if (open) {
      setCategory(null);
      setNote("");
    }
  }, [open]);

  const submit = () => {
    if (!category || submitReport.isPending) return;
    submitReport.mutate(
      {
        authorPubkey,
        eventId,
        reportType: category,
        note: note.trim() || undefined,
      },
      {
        onSuccess: () => {
          toast.success(t("moderation.report.submitted"));
          onOpenChange(false);
        },
        onError: () => toast.error(t("moderation.report.submit-failed")),
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Flag className="h-4 w-4" />
            {t("messages.action.report-message")}
          </DialogTitle>
          <DialogDescription>
            {t("moderation.report.description")}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          {REPORT_CATEGORIES.map((item) => (
            <Button
              key={item.value}
              variant={category === item.value ? "default" : "outline"}
              className="justify-start"
              disabled={submitReport.isPending}
              onClick={() => setCategory(item.value)}
            >
              {item.label}
            </Button>
          ))}
        </div>

        <div className="space-y-2">
          <label
            htmlFor="report-note"
            className="text-sm font-medium text-muted-foreground"
          >
            {t("moderation.report.additional-context")}
          </label>
          <Textarea
            id="report-note"
            placeholder={t("moderation.report.note-placeholder")}
            value={note}
            onChange={(e) => setNote(e.target.value)}
            rows={3}
            className="resize-none"
          />
        </div>

        <DialogFooter>
          <Button
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={submitReport.isPending}
          >
            {t("moderation.report.cancel")}
          </Button>
          <Button
            variant="default"
            onClick={submit}
            disabled={!category || submitReport.isPending}
          >
            {t("moderation.report.submit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
