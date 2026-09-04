import {
  AlertDialog,
  AlertDialogTrigger,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/shared/ui/alert-dialog";
import type { TriggerState } from "../triggerOperations";
import { Button } from "@/shared/ui/button";

export function WorkflowTriggerFeedback({
  state,
  onRetry,
  onNewRun,
}: {
  state: TriggerState;
  onRetry: () => void;
  onNewRun: () => void;
}) {
  if (state.status === "idle") return null;
  if (state.status === "pending")
    return (
      <p role="status" className="text-xs">
        Triggering workflow…
      </p>
    );
  if (state.status === "success")
    return (
      <p role="status" className="text-xs">
        Run created: {state.result?.runId}
      </p>
    );
  if (state.failurePhase === "prepare")
    return (
      <div
        role="alert"
        className="pointer-events-auto space-y-2 text-xs text-destructive"
      >
        <p>Workflow run was not started. {state.error}</p>
        <p>Retry prepares a new request before submitting it.</p>
        <Button size="sm" variant="outline" onClick={onRetry}>
          Retry trigger
        </Button>
      </div>
    );
  return (
    <div
      role="alert"
      className="pointer-events-auto space-y-2 text-xs text-destructive"
    >
      <p>Could not confirm workflow run. {state.error}</p>
      <p>
        The run may already exist. Retry reuses the same signed request.
        Recovery is available only while this app stays open.
      </p>
      <Button size="sm" variant="outline" onClick={onRetry}>
        Retry trigger
      </Button>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button size="sm" variant="outline">
            Start a distinct run…
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Start another run?</AlertDialogTitle>
            <AlertDialogDescription>
              The previous request may already have created a run. Starting a
              distinct run can repeat its side effects and discards the previous
              retry request. Prefer Retry trigger if you want only one run.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={onNewRun}>
              Start distinct run
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
