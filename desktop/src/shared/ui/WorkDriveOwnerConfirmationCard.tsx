import * as React from "react";
import { CheckCircle2, FolderUp } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import type { WorkDriveOwnerConfirmation } from "@/shared/lib/ownerConfirmation";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type ConfirmationResponse = { event_id: string };

export function WorkDriveOwnerConfirmationCard({
  channelId,
  requestEventId,
  payload,
}: {
  channelId: string;
  requestEventId: string;
  payload: WorkDriveOwnerConfirmation;
}) {
  const [submitting, setSubmitting] = React.useState(false);
  const [confirmedEventId, setConfirmedEventId] = React.useState<string | null>(
    null,
  );

  const confirm = async () => {
    setSubmitting(true);
    try {
      const response = await invokeTauri<ConfirmationResponse>(
        "confirm_workdrive_owner_request",
        { requestEventId, channelId },
      );
      setConfirmedEventId(response.event_id);
      toast.success("WorkDrive upload permission approved");
    } catch (error) {
      toast.error(
        `Approval failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="mt-2 max-w-lg rounded-xl border border-border bg-muted/35 p-4">
      <div className="flex items-start gap-3">
        {confirmedEventId ? (
          <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-emerald-500" />
        ) : (
          <FolderUp className="mt-0.5 h-5 w-5 shrink-0 text-primary" />
        )}
        <div className="min-w-0 flex-1">
          <p className="font-medium">
            {confirmedEventId
              ? "WorkDrive access approved"
              : "Approve WorkDrive upload access?"}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            Adds create-only file upload to this channel’s existing connection.
            Existing files cannot be overwritten, moved, shared, or deleted.
          </p>
          <div className="mt-3 text-xs text-muted-foreground">
            Profile:{" "}
            <span className="font-mono">{payload.capability_profile}</span>
          </div>
          {!confirmedEventId ? (
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button className="mt-3" size="sm">
                  Review and approve
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>
                    Allow create-only WorkDrive uploads?
                  </AlertDialogTitle>
                  <AlertDialogDescription>
                    This signs the exact approval shown here for this channel
                    and existing connection. Cancel creates no approval. Any
                    changed channel, owner, scope, operation, or connection is
                    rejected.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <div className="rounded-lg bg-muted p-3 text-sm">
                  <p>
                    <strong>Allowed:</strong> create one requested file in the
                    bound folder
                  </p>
                  <p className="mt-1">
                    <strong>Not allowed:</strong> overwrite, move, share, or
                    delete
                  </p>
                </div>
                <AlertDialogFooter>
                  <AlertDialogCancel disabled={submitting}>
                    Cancel
                  </AlertDialogCancel>
                  <AlertDialogAction
                    disabled={submitting}
                    onClick={() => void confirm()}
                  >
                    {submitting ? "Approving…" : "Approve"}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          ) : null}
        </div>
      </div>
    </div>
  );
}
