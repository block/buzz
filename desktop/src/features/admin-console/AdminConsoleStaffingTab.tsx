/**
 * Staffing tab — Operator-only UI for managing relay_operators rows.
 *
 * Source badges distinguish config-backed entries (immutable via API) from
 * DB-managed entries (can be added/removed/updated). 409 conflicts from the
 * server (config-backed key modification attempts) are surfaced with a clear
 * message.
 *
 * Display-name resolution follows the same pattern as the Invites surface:
 * names come from `useUsersBatchQuery`; hovering a name cross-fades to the
 * truncated npub so the raw identity is always one interaction away.
 */

import { useState } from "react";
import { nip19 } from "nostr-tools";
import { LoaderCircle, Trash2 } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileSummary } from "@/shared/api/types";
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
import {
  deleteAdminOperator,
  listAdminOperators,
  putAdminOperator,
  type AdminOperatorDto,
} from "./api";
import {
  type AsyncState,
  ErrorMessage,
  LoadingSpinner,
  useAsyncLoad,
} from "./AdminConsolePanelHelpers";

// ── Display-name helpers (mirrors CommunityMembersSettingsCard) ───────────

function formatDisplayName(
  pubkey: string,
  profile?: UserProfileSummary | null,
): string {
  const trimmed = profile?.displayName?.trim();
  if (trimmed && !trimmed.toLowerCase().startsWith("npub1")) {
    return trimmed;
  }
  return truncatePubkey(pubkey);
}

function npubFromPubkey(pubkey: string): string | null {
  try {
    return nip19.npubEncode(pubkey);
  } catch {
    return null;
  }
}

/**
 * Cross-fades between display name and npub on hover — same animation as the
 * Invites / CommunityMembersSettingsCard surface so the two feel identical.
 */
function HoverStaffingIdentity({
  pubkey,
  displayName,
}: {
  pubkey: string;
  displayName: string;
}) {
  const npub = npubFromPubkey(pubkey) ?? pubkey;
  return (
    <span className="inline-grid min-w-0 max-w-full grid-cols-1" title={npub}>
      <span
        className="col-start-1 row-start-1 max-w-40 truncate opacity-100 blur-0 transition-[max-width,opacity,filter] duration-[250ms] ease-in-out group-hover/staffing-row:max-w-0 group-hover/staffing-row:opacity-0 group-hover/staffing-row:blur-[2px] motion-reduce:transition-none"
        data-testid={`staffing-name-${pubkey}`}
      >
        {displayName}
      </span>
      <span
        className="col-start-1 row-start-1 max-w-0 truncate font-mono text-2xs opacity-0 blur-0 transition-[max-width,opacity,filter] duration-[250ms] ease-in-out group-hover/staffing-row:max-w-40 group-hover/staffing-row:opacity-100 group-hover/staffing-row:blur-0 motion-reduce:transition-none"
        data-testid={`staffing-npub-${pubkey}`}
      >
        {truncatePubkey(npub)}
      </span>
    </span>
  );
}

// ── Source badge ──────────────────────────────────────────────────────────

/** Source badge for an operator entry. */
function SourceBadge({
  source,
}: {
  source: "config" | "owner_fallback" | "db";
}) {
  const label: Record<string, string> = {
    config: "config",
    owner_fallback: "owner (fallback)",
    db: "db",
  };
  const variant: Record<string, "secondary" | "outline"> = {
    config: "secondary",
    owner_fallback: "secondary",
    db: "outline",
  };
  return (
    <Badge variant={variant[source] ?? "outline"}>
      {label[source] ?? source}
    </Badge>
  );
}

// ── Staffing tab ──────────────────────────────────────────────────────────

export function StaffingTab({
  origin,
  pubkey,
  generation,
  canMutate,
}: {
  origin: string;
  pubkey: string;
  generation: number;
  /**
   * When false (disabled-auth probe), all write affordances are hidden.
   * The operator list is still readable; only add/remove/edit controls are absent.
   */
  canMutate: boolean;
}) {
  const [listGen, setListGen] = useState(0);
  const [addPubkey, setAddPubkey] = useState("");
  const [addRole, setAddRole] = useState<"operator" | "moderator">("moderator");
  const [isAdding, setIsAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [workingPubkey, setWorkingPubkey] = useState<string | null>(null);
  /** Operator pending removal confirmation; null when dialog is closed. */
  const [pendingRemove, setPendingRemove] = useState<AdminOperatorDto | null>(
    null,
  );

  const listState: AsyncState<AdminOperatorDto[]> = useAsyncLoad(
    () => listAdminOperators(origin),
    [origin, pubkey],
    generation + listGen,
  );

  // Resolve display names using the shared hook — normalised delta-fetch with
  // per-pubkey caching, persisted-label seeding, and focus-recovery retry.
  // CommunitiesProvider wraps the entire Settings tree (main.tsx) so this is
  // always in context.
  const listedPubkeys =
    listState.status === "ok" ? listState.data.map((op) => op.pubkey) : [];
  const profilesQuery = useUsersBatchQuery(listedPubkeys, {
    enabled: listedPubkeys.length > 0,
  });

  const handleAdd = async () => {
    const trimmed = addPubkey.trim().toLowerCase();
    if (!trimmed) return;
    // Enforce create-only invariant: reject if the roster hasn't loaded —
    // the disabled button is the primary UI gate, but this guard closes the
    // boundary at the mutation call site itself.
    if (listState.status !== "ok") return;
    setAddError(null);

    // Reject any pubkey already present in the authoritative roster.
    const existing = listState.data.find((op) => op.pubkey === trimmed);
    if (existing) {
      setAddError(
        `Already an operator: ${existing.effectiveRole}. Change role with the edit control on their row.`,
      );
      return;
    }

    setIsAdding(true);
    try {
      await putAdminOperator(origin, trimmed, addRole);
      setAddPubkey("");
      setListGen((g) => g + 1);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // 409 = config-backed key; surface clearly
      setAddError(
        msg.includes("409")
          ? "This pubkey is config-backed and cannot be changed via the API."
          : msg,
      );
    } finally {
      setIsAdding(false);
    }
  };

  const handleRoleChange = async (
    op: AdminOperatorDto,
    newRole: "operator" | "moderator",
  ) => {
    if (newRole === op.effectiveRole) return;
    setActionError(null);
    setWorkingPubkey(op.pubkey);
    try {
      await putAdminOperator(origin, op.pubkey, newRole);
      setListGen((g) => g + 1);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setActionError(
        msg.includes("409")
          ? `Cannot change ${truncatePubkey(op.pubkey)}: config-backed key.`
          : msg,
      );
    } finally {
      setWorkingPubkey(null);
    }
  };

  const handleConfirmRemove = async () => {
    const op = pendingRemove;
    if (!op) return;
    setPendingRemove(null);
    setActionError(null);
    setWorkingPubkey(op.pubkey);
    try {
      await deleteAdminOperator(origin, op.pubkey);
      setListGen((g) => g + 1);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setActionError(
        msg.includes("409")
          ? `Cannot remove ${truncatePubkey(op.pubkey)}: config-backed key.`
          : msg,
      );
    } finally {
      setWorkingPubkey(null);
    }
  };

  const isSelf = pendingRemove?.pubkey === pubkey;

  return (
    <div className="space-y-4" data-testid="staffing-tab">
      {/* Remove confirmation dialog */}
      <AlertDialog
        open={pendingRemove !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRemove(null);
        }}
      >
        <AlertDialogContent data-testid="staffing-remove-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Remove operator?</AlertDialogTitle>
            <AlertDialogDescription asChild>
              <div className="space-y-2">
                <p>
                  This will remove{" "}
                  <span className="font-mono">
                    {pendingRemove ? truncatePubkey(pendingRemove.pubkey) : ""}
                  </span>{" "}
                  ({pendingRemove?.effectiveRole}) from the operator list.
                </p>
                {isSelf && (
                  <p
                    className="text-destructive"
                    data-testid="staffing-remove-self-warning"
                  >
                    You are removing your own operator access. Once removed, you
                    may lose the ability to undo this action.
                  </p>
                )}
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="staffing-remove-cancel">
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction asChild data-testid="staffing-remove-confirm">
              <Button
                onClick={() => void handleConfirmRemove()}
                variant="destructive"
              >
                Remove
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Add operator form — hidden in read-only (disabled-auth) mode */}
      {canMutate && (
        <div className="rounded-md border border-border/60 px-3 py-2.5 space-y-2">
          <p className="text-xs font-medium text-muted-foreground">
            Add operator
          </p>
          <div className="flex gap-2">
            <input
              className="flex-1 rounded-md border border-border/60 bg-background px-2 py-1 text-xs font-mono"
              data-testid="staffing-add-pubkey-input"
              disabled={isAdding}
              onChange={(e) => setAddPubkey(e.target.value)}
              placeholder="64-hex pubkey"
              type="text"
              value={addPubkey}
            />
            <select
              className="rounded-md border border-border/60 bg-background px-2 py-1 text-xs"
              data-testid="staffing-add-role-select"
              disabled={isAdding}
              onChange={(e) =>
                setAddRole(e.target.value as "operator" | "moderator")
              }
              value={addRole}
            >
              <option value="moderator">moderator</option>
              <option value="operator">operator</option>
            </select>
            <Button
              data-testid="staffing-add-btn"
              disabled={
                isAdding || !addPubkey.trim() || listState.status !== "ok"
              }
              onClick={() => void handleAdd()}
              size="sm"
              type="button"
            >
              {isAdding ? (
                <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
              ) : (
                "Add"
              )}
            </Button>
          </div>
          {addError && <p className="text-xs text-destructive">{addError}</p>}
        </div>
      )}

      {/* Operator list */}
      {listState.status === "loading" && <LoadingSpinner />}
      {listState.status === "error" && (
        <ErrorMessage message={listState.message} />
      )}
      {actionError && <ErrorMessage message={actionError} />}
      {listState.status === "ok" && (
        <ul className="space-y-1">
          {listState.data.length === 0 && (
            <p className="text-sm text-muted-foreground">
              No operators configured.
            </p>
          )}
          {listState.data.map((op: AdminOperatorDto) => {
            const isConfigBacked = op.sources.some(
              (s) => s === "config" || s === "owner_fallback",
            );
            const isWorking = workingPubkey === op.pubkey;
            const profile = profilesQuery.data?.profiles[op.pubkey];
            const displayName = formatDisplayName(op.pubkey, profile);
            return (
              <li
                className="group/staffing-row flex items-center gap-2 rounded-md border border-border/60 px-3 py-2"
                data-testid={`staffing-row-${op.pubkey}`}
                key={op.pubkey}
              >
                <div className="flex-1 min-w-0">
                  <p className="text-xs truncate">
                    <HoverStaffingIdentity
                      pubkey={op.pubkey}
                      displayName={displayName}
                    />
                  </p>
                  <div className="flex flex-wrap gap-1 mt-0.5">
                    {op.sources.map((s) => (
                      <SourceBadge key={s} source={s} />
                    ))}
                  </div>
                </div>
                {/* In-place role selector — hidden in read-only / config-backed mode */}
                {canMutate && !isConfigBacked && (
                  <select
                    aria-label={`Change role for ${displayName}`}
                    className="rounded-md border border-border/60 bg-background px-1.5 py-0.5 text-xs"
                    data-testid={`staffing-role-select-${op.pubkey}`}
                    disabled={isWorking}
                    onChange={(e) =>
                      void handleRoleChange(
                        op,
                        e.target.value as "operator" | "moderator",
                      )
                    }
                    value={op.effectiveRole}
                  >
                    <option value="moderator">moderator</option>
                    <option value="operator">operator</option>
                  </select>
                )}
                {/* Config-backed: show role as badge (not editable) */}
                {isConfigBacked && (
                  <Badge variant="outline">{op.effectiveRole}</Badge>
                )}
                {/* Remove button — hidden in read-only (disabled-auth) mode */}
                {canMutate && (
                  <Button
                    aria-label={`Remove ${displayName}`}
                    data-testid={`staffing-remove-btn-${op.pubkey}`}
                    disabled={isConfigBacked || isWorking}
                    onClick={() => setPendingRemove(op)}
                    size="icon-xs"
                    title={
                      isConfigBacked
                        ? "Config-backed — cannot be removed via API"
                        : "Remove operator"
                    }
                    type="button"
                    variant="ghost"
                  >
                    {isWorking ? (
                      <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Trash2 className="h-3.5 w-3.5" />
                    )}
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
