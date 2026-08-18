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
  relayHttpUrl: string;
  webhookSecret: string;
  workflowId: string;
};

export function WorkflowWebhookSecretDialog({
  onOpenChange,
  open,
  relayHttpUrl,
  webhookSecret,
  workflowId,
}: WorkflowWebhookSecretDialogProps) {
  const webhookUrl = `${relayHttpUrl}/hooks/${workflowId}`;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Webhook Ready</DialogTitle>
          <DialogDescription>
            This secret is only shown now. If it is lost, re-save the workflow
            to generate a new one.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              Webhook URL
            </p>
            <pre className="overflow-x-auto rounded-md bg-muted/50 p-3 font-mono text-xs">
              {webhookUrl}
            </pre>
            <CopyButton label="Copy URL" value={webhookUrl} />
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
