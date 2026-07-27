import * as React from "react";
import { toast } from "sonner";

import type { ChannelMember } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";

type RecoveryMutation = {
  error: unknown;
  isPending: boolean;
  mutateAsync: (input: {
    reason: string;
    targetPubkey: string;
  }) => Promise<unknown>;
  reset: () => void;
};

export function ChannelOwnerRecoveryDialog({
  candidates,
  channelName,
  currentOwners,
  mutation,
  onOpenChange,
  open,
}: {
  candidates: ChannelMember[];
  channelName: string;
  currentOwners: string[];
  mutation: RecoveryMutation;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const [targetPubkey, setTargetPubkey] = React.useState("");
  const [reason, setReason] = React.useState("");
  const [confirmed, setConfirmed] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      setTargetPubkey("");
      setReason("");
      setConfirmed(false);
    } else if (!targetPubkey && candidates[0]) {
      setTargetPubkey(candidates[0].pubkey);
    }
  }, [candidates, open, targetPubkey]);

  const trimmedReason = reason.trim();
  const reasonByteLength = new TextEncoder().encode(trimmedReason).length;
  const canSubmit =
    targetPubkey.length === 64 &&
    reasonByteLength > 0 &&
    reasonByteLength <= 500 &&
    confirmed &&
    !mutation.isPending;
  const selectedTarget = candidates.find(
    (candidate) => candidate.pubkey === targetPubkey,
  );
  const selectedTargetName =
    selectedTarget?.displayName?.trim() || selectedTarget?.pubkey || "None";
  const ownerSummary =
    currentOwners.length > 0 ? currentOwners.join(", ") : "None listed";

  async function submit() {
    if (!canSubmit) {
      return;
    }
    try {
      await mutation.mutateAsync({
        targetPubkey,
        reason: trimmedReason,
      });
      toast.success("Channel owner recovered");
      onOpenChange(false);
    } catch {
      // Preserve the relay's authoritative denial in the inline error below.
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby="channel-owner-recovery-policy"
        className="max-h-[calc(100vh-2rem)] max-w-lg grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden"
        data-testid="channel-owner-recovery-dialog"
      >
        <DialogHeader>
          <DialogTitle>Recover channel owner</DialogTitle>
        </DialogHeader>

        <div className="min-h-0 space-y-4 overflow-y-auto pr-1">
          <p
            className="text-sm leading-6 text-muted-foreground"
            id="channel-owner-recovery-policy"
          >
            Every current human owner must have self-archived and named this
            target as their replacement. Recovery is denied while an active
            human admin or any owner/admin agent exists. A lost or deleted key
            without that durable consent is not recoverable.
          </p>

          <p className="rounded-xl border border-border/70 bg-muted/30 px-3 py-2 text-sm leading-5">
            This only promotes the selected member. It does not remove or demote
            anyone, and the channel ID, messages, threads, roster, canvas, and
            workflows remain unchanged.
          </p>

          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-xl border border-border/70 px-3 py-2 text-sm leading-5">
            <dt className="text-muted-foreground">Channel</dt>
            <dd className="min-w-0 break-words font-medium">#{channelName}</dd>
            <dt className="text-muted-foreground">Current owners</dt>
            <dd
              className="min-w-0 break-words font-medium"
              data-testid="channel-owner-recovery-current-owners"
            >
              {ownerSummary}
            </dd>
            <dt className="text-muted-foreground">Replacement</dt>
            <dd className="min-w-0 break-words font-medium">
              {selectedTargetName}
            </dd>
            <dt className="text-muted-foreground">Audit reason</dt>
            <dd className="min-w-0 break-words font-medium">
              {trimmedReason || "Enter a reason below"}
            </dd>
          </dl>

          <label
            className="block space-y-1.5 text-sm font-medium"
            htmlFor="channel-owner-recovery-target"
          >
            <span>Replacement owner</span>
            <select
              className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
              data-testid="channel-owner-recovery-target"
              disabled={mutation.isPending}
              id="channel-owner-recovery-target"
              onChange={(event) => setTargetPubkey(event.target.value)}
              value={targetPubkey}
            >
              {candidates.map((candidate) => (
                <option key={candidate.pubkey} value={candidate.pubkey}>
                  {candidate.displayName?.trim() || candidate.pubkey}
                </option>
              ))}
            </select>
          </label>

          <label
            className="block space-y-1.5 text-sm font-medium"
            htmlFor="channel-owner-recovery-reason"
          >
            <span>Audit reason</span>
            <Textarea
              data-testid="channel-owner-recovery-reason"
              disabled={mutation.isPending}
              id="channel-owner-recovery-reason"
              maxLength={500}
              onChange={(event) => setReason(event.target.value)}
              placeholder="Why this recovery is necessary"
              value={reason}
            />
            {reasonByteLength > 500 ? (
              <span className="text-xs text-destructive">
                Audit reason must be at most 500 UTF-8 bytes.
              </span>
            ) : null}
          </label>

          <label className="flex items-start gap-3 text-sm leading-5">
            <input
              checked={confirmed}
              className="mt-1"
              data-testid="channel-owner-recovery-confirm"
              disabled={mutation.isPending}
              onChange={(event) => setConfirmed(event.target.checked)}
              type="checkbox"
            />
            <span>
              I confirm recovery for #{channelName}, promoting{" "}
              {selectedTargetName}, with the current owners and audit reason
              shown above. The relay will apply the prior-self-consent predicate
              and record an immutable channel-visible audit event.
            </span>
          </label>

          {mutation.error instanceof Error ? (
            <p
              className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              data-testid="channel-owner-recovery-error"
            >
              {mutation.error.message}
            </p>
          ) : null}
        </div>

        <div className="flex justify-end gap-2">
          <Button
            disabled={mutation.isPending}
            onClick={() => onOpenChange(false)}
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            data-testid="channel-owner-recovery-submit"
            disabled={!canSubmit}
            onClick={() => void submit()}
            type="button"
          >
            {mutation.isPending ? "Recovering..." : "Recover owner"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
