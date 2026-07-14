import { MessageCircle, SlidersHorizontal } from "lucide-react";

import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type WelcomeAgentCreateDialogProps = {
  guideName: string;
  isSending: boolean;
  open: boolean;
  sendError?: string | null;
  onCreateInChat: () => void;
  onCreateManually: () => void;
  onOpenChange: (open: boolean) => void;
};

export function WelcomeAgentCreateDialog({
  guideName,
  isSending,
  open,
  sendError,
  onCreateInChat,
  onCreateManually,
  onOpenChange,
}: WelcomeAgentCreateDialogProps) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Create an agent</DialogTitle>
          <DialogDescription>
            Start with a conversation, or set everything up yourself.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-2">
          <button
            className="flex w-full items-start gap-3 rounded-lg border border-primary bg-primary/5 p-4 text-left transition-colors hover:bg-primary/10 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
            data-testid="welcome-create-agent-in-chat"
            disabled={isSending}
            onClick={onCreateInChat}
            type="button"
          >
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
              <MessageCircle className="h-5 w-5" />
            </span>
            <span className="min-w-0">
              <span className="block text-sm font-medium text-foreground">
                Create with {guideName}
              </span>
              <span className="mt-0.5 block text-sm text-muted-foreground">
                Talk through what you need. {guideName} will prepare a draft you
                can review and edit.
              </span>
            </span>
          </button>

          <button
            className="flex w-full items-start gap-3 rounded-lg p-4 text-left transition-colors hover:bg-accent focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="welcome-create-agent-manually"
            onClick={onCreateManually}
            type="button"
          >
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-foreground">
              <SlidersHorizontal className="h-5 w-5" />
            </span>
            <span className="min-w-0">
              <span className="block text-sm font-medium text-foreground">
                Create manually
              </span>
              <span className="mt-0.5 block text-sm text-muted-foreground">
                Fill in the agent’s name, instructions, and settings yourself.
              </span>
            </span>
          </button>
        </div>

        {sendError ? (
          <p className="text-sm text-destructive" role="alert">
            {sendError}
          </p>
        ) : null}

        <div className="flex justify-end">
          <Button
            disabled={isSending}
            onClick={() => onOpenChange(false)}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
