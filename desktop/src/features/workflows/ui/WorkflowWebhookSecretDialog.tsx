import { Eye, EyeOff } from "lucide-react";
import { useTranslation } from "@/i18n";
import * as React from "react";

import { CopyButton } from "@/features/agents/ui/CopyButton";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type WorkflowWebhookSecretDialogProps = {
  onContinue: () => void;
  open: boolean;
  relayHttpUrl: string | null;
  relayUrlError: string | null;
  webhookSecret: string;
  workflowId: string;
};

export function WorkflowWebhookSecretDialog({
  onContinue,
  open,
  relayHttpUrl,
  relayUrlError,
  webhookSecret,
  workflowId,
}: WorkflowWebhookSecretDialogProps) {
  const { t } = useTranslation();
  const [revealed, setRevealed] = React.useState(false);
  const webhookUrl = relayHttpUrl
    ? `${relayHttpUrl}/hooks/${workflowId}`
    : null;

  return (
    <Dialog onOpenChange={(nextOpen) => !nextOpen && onContinue()} open={open}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("workflows.webhook.ready-title")}</DialogTitle>

          <DialogDescription>
            {t("workflows.webhook.ready-description")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              {t("workflows.webhook.url-label")}
            </p>
            {webhookUrl ? (
              <>
                <pre className="overflow-x-auto rounded-md bg-muted/50 p-3 font-mono text-xs">
                  {webhookUrl}
                </pre>
                <CopyButton
                  label={t("workflows.webhook.copy-url")}
                  value={webhookUrl}
                />
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
                  ? t("workflows.webhook.url-load-error", {
                      error: relayUrlError,
                    })
                  : t("workflows.webhook.url-loading")}
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              `X-Webhook-Secret`
            </p>
            <div className="flex items-center gap-2 rounded-md bg-muted/50 p-3">
              <code className="min-w-0 flex-1 overflow-x-auto font-mono text-xs">
                {revealed ? webhookSecret : "•".repeat(24)}
              </code>
              <Button
                aria-label={
                  revealed
                    ? t("workflows.webhook.hide-secret")
                    : t("workflows.webhook.reveal-secret")
                }
                onClick={() => setRevealed((value) => !value)}
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                {revealed ? <EyeOff /> : <Eye />}
              </Button>
            </div>
            <CopyButton
              label={t("workflows.webhook.copy-secret")}
              value={webhookSecret}
            />
          </div>
        </div>
        <DialogFooter>
          <Button onClick={onContinue} type="button">
            {t("workflows.dialog.continue")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
