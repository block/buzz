import type { FizzAgentManagementRequest } from "@/features/agents/fizzAgentManagement";
import type { AgentPersona, RespondToMode } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

function accessLabel(value: RespondToMode | undefined) {
  if (value === "anyone") return "Anyone in this workspace";
  if (value === "allowlist") return "Specific people";
  return "Just me";
}

function ChangeRow({ label, value }: { label: string; value?: string }) {
  if (!value) return null;
  return (
    <div className="grid gap-1 border-b border-border/60 py-3 last:border-0">
      <dt className="text-xs font-medium text-muted-foreground">{label}</dt>
      <dd className="whitespace-pre-wrap text-sm text-foreground">{value}</dd>
    </div>
  );
}

type Props = {
  open: boolean;
  request: FizzAgentManagementRequest | null;
  existingPersona?: AgentPersona;
  isPending: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
};

/** Owner-confirmed boundary for agent changes requested by Fizz. */
export function FizzAgentManagementReview({
  open,
  request,
  existingPersona,
  isPending,
  error,
  onOpenChange,
  onConfirm,
}: Props) {
  if (!request) return null;
  const creating = request.action === "create";
  const details = request.request;
  const name = creating
    ? details.displayName
    : (details.displayName ?? existingPersona?.displayName ?? "this agent");

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {creating ? `Create ${name}?` : `Update ${name}?`}
          </DialogTitle>
          <DialogDescription>
            Fizz prepared this request. Nothing changes until you confirm it.
          </DialogDescription>
        </DialogHeader>

        <dl className="rounded-md border border-border bg-muted/20 px-4">
          <ChangeRow label="Why Fizz suggests this" value={details.rationale} />
          {creating ? (
            <ChangeRow label="Name" value={details.displayName} />
          ) : null}
          <ChangeRow label="Instructions" value={details.systemPrompt} />
          <ChangeRow label="Runtime" value={details.runtime} />
          <ChangeRow label="Provider" value={details.provider} />
          <ChangeRow label="Model" value={details.model} />
          {details.respondTo ? (
            <ChangeRow
              label="Who can ask it for help"
              value={accessLabel(details.respondTo)}
            />
          ) : null}
        </dl>

        {details.respondTo === "anyone" ? (
          <p className="text-sm leading-6 text-muted-foreground">
            Anyone in this workspace can invoke this agent. Their requests may
            use your configured model access.
          </p>
        ) : null}
        <p className="text-xs leading-5 text-muted-foreground">
          Provider keys and other credentials are never included in Fizz
          requests. If this setup needs one, Buzz will ask for it privately
          after you confirm.
        </p>
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        <div className="flex justify-end gap-2">
          <Button
            disabled={isPending}
            onClick={() => onOpenChange(false)}
            variant="ghost"
          >
            Cancel
          </Button>
          <Button disabled={isPending} onClick={onConfirm}>
            {isPending ? "Saving…" : creating ? "Create agent" : "Save changes"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
