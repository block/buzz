import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type ExternalAgentAuthorizationDialogProps = {
  agentLabel: string;
  error: unknown;
  isPending: boolean;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
};

export function ExternalAgentAuthorizationDialog({
  agentLabel,
  error,
  isPending,
  onConfirm,
  onOpenChange,
  open,
}: ExternalAgentAuthorizationDialogProps) {
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent data-testid="external-agent-authorization-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Authorize external agent?</AlertDialogTitle>
          <AlertDialogDescription>
            Only continue if you control and trust {agentLabel}. Buzz will sign
            an owner authorization that lets this agent use your relay
            membership and identifies it as managed by you. Your private key
            stays on this device; only the resulting authorization is copied.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error instanceof Error ? (
          <p className="text-sm text-destructive">{error.message}</p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={isPending} type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid="external-agent-authorization-confirm"
              disabled={isPending}
              onClick={(event) => {
                event.preventDefault();
                onConfirm();
              }}
              type="button"
            >
              {isPending ? "Authorizing..." : "Authorize and copy"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
