import React from "react";
import { toast } from "sonner";

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
import { Checkbox } from "@/shared/ui/checkbox";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { isRelocationOfLocalAgent } from "../agentRelocation";
import {
  approveAttestation,
  declineAttestation,
  usePendingAttestations,
  type PendingAttestation,
} from "../spawnerAttestationStore";

/** Shorten a 64-char pubkey for display without losing recognizability. */
export function shortenPubkey(pubkey: string): string {
  if (pubkey.length <= 20) return pubkey;
  return `${pubkey.slice(0, 10)}…${pubkey.slice(-6)}`;
}

/**
 * Consent copy for an attestation request.
 *
 * Pure so the wording stays unit-testable. It has to say plainly what a
 * signature does — this is not "allow an app to run", it is "let this key act
 * in your community under your membership" — because a user who misreads it
 * grants relay access to a server they may not control.
 */
export function attestationDescription(
  item: PendingAttestation | null,
  options?: { isRelocation?: boolean },
): string {
  if (!item) return "A server wants to run an agent for you.";
  if (options?.isRelocation) {
    // A relocation is a different decision from authorizing a stranger: the
    // key already belongs to the user, and what they are consenting to is
    // handing it to the server and giving up the local copy. Reusing the "a
    // new agent key was created" copy here would be a lie in both halves.
    return (
      `The spawner ${shortenPubkey(item.spawnerPubkey)} is asking to take over ` +
      `"${item.specSlug}", an agent you already run on this Mac.\n\n` +
      `Approving moves ${shortenPubkey(item.agentPubkey)} to that server. It keeps ` +
      `the same identity — its channels, its profile, and its memory all follow ` +
      `it — and it will stop running on this Mac. ` +
      `Only approve this if you run this server.`
    );
  }
  return (
    `The spawner ${shortenPubkey(item.spawnerPubkey)} created a new agent key ` +
    `for "${item.specSlug}" and is asking you to authorize it.\n\n` +
    `Approving signs an owner attestation for ${shortenPubkey(item.agentPubkey)}, ` +
    `which lets that key join and read your channels as an agent you own. ` +
    `Only approve this if you run this server.`
  );
}

/** Dialog title, which also has to change for a move. */
export function attestationTitle(options?: { isRelocation?: boolean }): string {
  return options?.isRelocation
    ? "Move this agent to a server?"
    : "Authorize a server agent?";
}

type SpawnerAttestationDialogProps = {
  /** Injectable for tests; defaults to the live queue. */
  pending?: readonly PendingAttestation[];
};

/**
 * Prompts the owner to approve or decline a server-hosted agent's key.
 *
 * Rendered app-wide so a request that arrives while the user is anywhere in the
 * app still surfaces. It shows one request at a time — the head of the queue —
 * because each is a distinct consent decision and stacking them invites
 * click-through.
 */
export function SpawnerAttestationDialog({
  pending: injectedPending,
}: SpawnerAttestationDialogProps = {}) {
  const livePending = usePendingAttestations();
  const pending = injectedPending ?? livePending;
  const current = pending[0] ?? null;

  // A request naming an agent this device already manages is a *move*, not a
  // new key. Read from the managed-agent list rather than trusting anything in
  // the frame: only local state can say whether we already own this identity.
  const managedAgentsQuery = useManagedAgentsQuery();
  const isRelocation = isRelocationOfLocalAgent(
    current?.agentPubkey,
    React.useMemo(
      () => (managedAgentsQuery.data ?? []).map((agent) => agent.pubkey),
      [managedAgentsQuery.data],
    ),
  );

  const [isSubmitting, setIsSubmitting] = React.useState(false);

  // Each request is its own consent decision, so "trust this spawner" must not
  // carry over from a previous prompt the user happened to tick. Storing the
  // nonce alongside the value resets it during render (React's "adjusting state
  // when a prop changes" pattern) rather than in an effect, so the checkbox is
  // never briefly shown ticked for the next request.
  const [rememberState, setRememberState] = React.useState<{
    nonce: string | null;
    value: boolean;
  }>({ nonce: null, value: false });
  const remember =
    rememberState.nonce === (current?.nonce ?? null)
      ? rememberState.value
      : false;
  const setRemember = (value: boolean) => {
    setRememberState({ nonce: current?.nonce ?? null, value });
  };

  const handleApprove = React.useCallback(async () => {
    if (!current || isSubmitting) return;
    setIsSubmitting(true);
    try {
      await approveAttestation(current, { remember });
      toast.success(
        isRelocation
          ? `Moved "${current.specSlug}" to the server.`
          : `Authorized the server agent "${current.specSlug}".`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to authorize the server agent.",
      );
    } finally {
      setIsSubmitting(false);
    }
  }, [current, isRelocation, isSubmitting, remember]);

  const handleDecline = React.useCallback(async () => {
    if (!current || isSubmitting) return;
    setIsSubmitting(true);
    try {
      await declineAttestation(current);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to decline the server agent.",
      );
    } finally {
      setIsSubmitting(false);
    }
  }, [current, isSubmitting]);

  return (
    <AlertDialog
      onOpenChange={(open) => {
        // Dismissing without a decision would leave the agent stuck at
        // pending_attestation until the spawner times out, so closing declines.
        if (!open) void handleDecline();
      }}
      open={current !== null}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {attestationTitle({ isRelocation })}
          </AlertDialogTitle>
          <AlertDialogDescription className="whitespace-pre-line">
            {attestationDescription(current, { isRelocation })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <label
          className="flex w-fit cursor-pointer items-center gap-2 text-sm text-muted-foreground"
          htmlFor="remember-spawner"
        >
          <Checkbox
            checked={remember}
            data-testid="remember-spawner"
            disabled={isSubmitting}
            id="remember-spawner"
            onCheckedChange={(checked) => setRemember(checked === true)}
          />
          Trust this spawner for future agents
        </label>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={isSubmitting} type="button" variant="outline">
              Decline
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              disabled={isSubmitting}
              onClick={(event) => {
                // Keep the dialog mounted until the signature is published, so
                // a failure surfaces on this prompt rather than after it closes.
                event.preventDefault();
                void handleApprove();
              }}
              type="button"
            >
              Approve
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
