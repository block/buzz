import * as React from "react";

import type { RegisteredAgentReference } from "@/shared/api/tauriRegisteredAgents";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

const REGISTER_EXISTING_AGENT_COPY =
  "Registers an existing identity for this device. Buzz will not import its key or run it.";

export function RegisterExistingAgentDialog({
  error,
  isPending,
  onOpenChange,
  onSubmit,
  open,
}: {
  error: Error | null;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (input: {
    pubkey: string;
    label?: string | null;
    roleSummary?: string | null;
  }) => Promise<RegisteredAgentReference>;
  open: boolean;
}) {
  const [pubkey, setPubkey] = React.useState("");
  const [label, setLabel] = React.useState("");
  const [roleSummary, setRoleSummary] = React.useState("");

  React.useEffect(() => {
    if (!open) {
      setPubkey("");
      setLabel("");
      setRoleSummary("");
    }
  }, [open]);

  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      await onSubmit({ pubkey, label, roleSummary });
      onOpenChange(false);
    } catch {
      // The mutation owns the error state rendered above. Keep the dialog open
      // and consume the rejection at this UI event boundary.
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent>
        <form className="space-y-4" onSubmit={(event) => void submit(event)}>
          <DialogHeader>
            <DialogTitle>Register existing agent</DialogTitle>
            <DialogDescription>
              {REGISTER_EXISTING_AGENT_COPY}
            </DialogDescription>
          </DialogHeader>
          <label
            className="block space-y-1.5 text-sm font-medium"
            htmlFor="registered-agent-pubkey"
          >
            <span>Public key</span>
            <Input
              id="registered-agent-pubkey"
              data-testid="register-existing-agent-pubkey"
              onChange={(event) => setPubkey(event.target.value)}
              placeholder="64-character public key"
              required
              value={pubkey}
            />
          </label>
          <label
            className="block space-y-1.5 text-sm font-medium"
            htmlFor="registered-agent-label"
          >
            <span>Label</span>
            <Input
              id="registered-agent-label"
              data-testid="register-existing-agent-label"
              onChange={(event) => setLabel(event.target.value)}
              placeholder="Optional display label"
              value={label}
            />
          </label>
          <label
            className="block space-y-1.5 text-sm font-medium"
            htmlFor="registered-agent-role-summary"
          >
            <span>Local role summary</span>
            <Input
              id="registered-agent-role-summary"
              data-testid="register-existing-agent-role-summary"
              onChange={(event) => setRoleSummary(event.target.value)}
              placeholder="Optional role summary"
              value={roleSummary}
            />
          </label>
          {error ? (
            <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error.message}
            </p>
          ) : null}
          <DialogFooter>
            <Button
              disabled={isPending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              disabled={isPending || pubkey.trim().length === 0}
              type="submit"
            >
              Register reference
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
