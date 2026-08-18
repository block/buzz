import { CopyButton } from "@/features/agents/ui/CopyButton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type WorkflowWebhookSecretDialogProps = {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  relayHttpUrl: string | null;
  relayUrlError: string | null;
  webhookSecret: string;
  workflowId: string;
};

export function WorkflowWebhookSecretDialog({
  onOpenChange,
  open,
  relayHttpUrl,
  relayUrlError,
  webhookSecret,
  workflowId,
}: WorkflowWebhookSecretDialogProps) {
  const webhookUrl = relayHttpUrl
    ? `${relayHttpUrl}/hooks/${workflowId}`
    : null;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Webhook Ready</DialogTitle>
          <DialogDescription>
            This secret is only shown now and cannot be recovered later. Copy it
            before closing this dialog.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              Webhook URL
            </p>
            {webhookUrl ? (
              <>
                <pre className="overflow-x-auto rounded-md bg-muted/50 p-3 font-mono text-xs">
                  {webhookUrl}
                </pre>
                <CopyButton label="Copy URL" value={webhookUrl} />
              </>
            ) : (
              <p
                className={
                  relayUrlError
                    ? "rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
                    : "rounded-md bg-muted/50 p-3 text-sm text-muted-foreground"
                }
              >
                {relayUrlError
                  ? `The workflow was saved, but its webhook URL could not be loaded: ${relayUrlError}`
                  : "Loading webhook URL…"}
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              `X-Webhook-Secret`
            </p>
            <pre className="overflow-x-auto rounded-md bg-muted/50 p-3 font-mono text-xs">
              {webhookSecret}
            </pre>
            <CopyButton label="Copy Secret" value={webhookSecret} />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
