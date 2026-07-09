import { Flag } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useSubmitReportMutation } from "@/features/moderation/hooks";
import type { ReportType } from "@/features/moderation/hooks";
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

/** NIP-56 report categories, in the order shown to the reporter. `other` is
 *  last so it reads as the fallback rather than a first-class choice. */
const REPORT_CATEGORIES: { value: ReportType; label: string }[] = [
  { value: "spam", label: "Spam" },
  { value: "profanity", label: "Profanity or hate speech" },
  { value: "nudity", label: "Nudity or sexual content" },
  { value: "impersonation", label: "Impersonation" },
  { value: "malware", label: "Malware or scam" },
  { value: "illegal", label: "Illegal content" },
  { value: "other", label: "Other" },
];

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
  const submitReport = useSubmitReportMutation();
  const [category, setCategory] = React.useState<ReportType | null>(null);
  const [note, setNote] = React.useState("");

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
          toast.success("Report submitted to community moderators");
          onOpenChange(false);
        },
        onError: () => toast.error("Failed to submit report"),
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Flag className="h-4 w-4" />
            Report message
          </DialogTitle>
          <DialogDescription>
            Reports go to this community's moderators for review. The author is
            not notified of who reported them.
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
            Additional context (optional)
          </label>
          <Textarea
            id="report-note"
            placeholder="Add anything that helps moderators..."
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
            Cancel
          </Button>
          <Button
            variant="default"
            onClick={submit}
            disabled={!category || submitReport.isPending}
          >
            Submit report
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
