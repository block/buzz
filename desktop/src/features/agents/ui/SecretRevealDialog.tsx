import type { AgentChannelAttachmentFailure } from "@/features/agents/channelAttachmentFailure";
import type { CreateManagedAgentResponse } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { CopyButton } from "./CopyButton";
import { ExternalAgentEnvBlock } from "./ExternalAgentEnvBlock";

export function SecretRevealDialog({
  attachmentFailure,
  created,
  isRetryingAttachment = false,
  onOpenChange,
  onRetryAttachment,
}: {
  attachmentFailure?: AgentChannelAttachmentFailure | null;
  created: CreateManagedAgentResponse | null;
  isRetryingAttachment?: boolean;
  onOpenChange: (open: boolean) => void;
  onRetryAttachment?: () => void;
}) {
  // External agents are started by the user, not by Buzz, so the useful
  // artifact is the whole container env — which already contains the nsec.
  // Showing the bare key alongside it would just duplicate the secret.
  const isExternal = created?.agent.backend.type === "external";

  return (
    <Dialog onOpenChange={onOpenChange} open={created !== null}>
      <DialogContent className="max-w-2xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Agent created</DialogTitle>
            <DialogDescription>
              {isExternal
                ? "Copy the environment below to start this agent where it lives. You can get it again later from the agent's settings."
                : "Save the private key now. The app can keep running the harness locally, but this secret is only revealed here."}
            </DialogDescription>
          </DialogHeader>

          <div className="flex-1 space-y-4 overflow-y-auto px-6 py-5">
            {created ? (
              <>
                {isExternal ? (
                  <ExternalAgentEnvBlock
                    agentName={created.agent.name}
                    pubkey={created.agent.pubkey}
                  />
                ) : (
                  <div className="rounded-2xl border border-border/70 bg-muted/20 p-4">
                    <div className="flex items-center justify-between gap-3">
                      <div>
                        <p className="text-sm font-semibold tracking-tight">
                          Private key (nsec)
                        </p>
                        <p className="text-sm text-muted-foreground">
                          This is the agent identity used by `buzz-acp`.
                        </p>
                      </div>
                      <CopyButton
                        label="Copy key"
                        value={created.privateKeyNsec}
                      />
                    </div>
                    <code className="mt-3 block break-all rounded-xl border border-border/70 bg-background/80 px-3 py-2 text-xs">
                      {created.privateKeyNsec}
                    </code>
                  </div>
                )}

                {created.profileSyncError ? (
                  <p className="rounded-2xl border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-warning">
                    {created.profileSyncError}
                  </p>
                ) : null}

                {created.spawnError ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {created.spawnError}
                  </p>
                ) : attachmentFailure ? (
                  <div
                    className="space-y-1 rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
                    role="alert"
                  >
                    <p>
                      {created.agent.name} was created, but couldn’t be added to
                      #{attachmentFailure.channelName}.
                    </p>
                    <p>{attachmentFailure.error}</p>
                  </div>
                ) : isExternal ? (
                  <p className="rounded-2xl border border-primary/20 bg-primary/10 px-4 py-3 text-sm text-primary">
                    {created.agent.name} has an identity and is in your
                    community. It will come online once you start it.
                  </p>
                ) : (
                  <p className="rounded-2xl border border-primary/20 bg-primary/10 px-4 py-3 text-sm text-primary">
                    {created.agent.name} is ready
                    {created.agent.status === "running"
                      ? " and running."
                      : created.agent.status === "deployed"
                        ? " and deployed."
                        : "."}
                  </p>
                )}
              </>
            ) : null}
          </div>

          <div className="flex justify-end gap-2 border-t border-border/60 px-6 py-4">
            {attachmentFailure && onRetryAttachment ? (
              <Button
                disabled={isRetryingAttachment}
                onClick={onRetryAttachment}
                size="sm"
                type="button"
              >
                {isRetryingAttachment ? "Trying again…" : "Try again"}
              </Button>
            ) : null}
            <Button
              disabled={isRetryingAttachment}
              onClick={() => onOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Done
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
