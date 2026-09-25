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
 *
 * A "Restrictions" section below the operator list lets operators lift active
 * bans and timeouts for the currently active community.
 */

import { useEffect, useRef, useState } from "react";
import { nip19 } from "nostr-tools";
import { LoaderCircle, Trash2 } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { getRelayWsUrl } from "@/shared/api/tauri";
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
  listAdminRestrictions,
  liftAdminBan,
  liftAdminTimeout,
  putAdminOperator,
  type AdminMemberRestrictionDto,
  type AdminOperatorDto,
} from "./api";
import {
  adminErrorMessage,
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

// ── Restrictions section ──────────────────────────────────────────────────

/**
 * Restriction type label for a row. A row can be banned, timed-out, or both.
 */
function RestrictionTypeBadge({
  record,
}: {
  record: AdminMemberRestrictionDto;
}) {
  const now = new Date();
  const isBanned =
    record.banned &&
    (record.banExpiresAt === null || new Date(record.banExpiresAt) > now);
  const isTimedOut =
    record.mutedUntil !== null && new Date(record.mutedUntil) > now;
  return (
    <span className="flex flex-wrap gap-1">
      {isBanned && <Badge variant="destructive">banned</Badge>}
      {isTimedOut && <Badge variant="secondary">timeout</Badge>}
    </span>
  );
}

/**
 * Active restrictions for the active relay's community. The native commands
 * name the community by the active relay's host, so there is no client-side
 * community id to get wrong; an unresolvable host surfaces as an error.
 * Operators can lift bans and timeouts per member row.
 */
function RestrictionsSection({
  origin,
  relayKey,
  generation,
}: {
  origin: string;
  /** Active relay identity; a change reloads the list. */
  relayKey: string;
  generation: number;
}) {
  const [listGen, setListGen] = useState(0);
  const [liftError, setLiftError] = useState<string | null>(null);
  const [workingPubkey, setWorkingPubkey] = useState<string | null>(null);
  /** Row pending a lift-ban confirmation. */
  const [pendingLiftBan, setPendingLiftBan] =
    useState<AdminMemberRestrictionDto | null>(null);
  /** Row pending a lift-timeout confirmation. */
  const [pendingLiftTimeout, setPendingLiftTimeout] =
    useState<AdminMemberRestrictionDto | null>(null);

  // The list captures the native relay it loaded from; every later page and
  // removal carries it so a relay switch fails loudly instead of retargeting.
  const listState: AsyncState<{
    relay: string;
    items: AdminMemberRestrictionDto[];
    nextCursor: string | null;
  }> = useAsyncLoad(
    async () => {
      const relay = await getRelayWsUrl();
      return { relay, ...(await listAdminRestrictions(origin, relay)) };
    },
    [origin, relayKey],
    generation + listGen,
  );
  const loadedRelay = listState.status === "ok" ? listState.data.relay : "";

  const handleConfirmLiftBan = async () => {
    const row = pendingLiftBan;
    if (!row) return;
    setPendingLiftBan(null);
    setLiftError(null);
    setWorkingPubkey(row.pubkey);
    try {
      await liftAdminBan(origin, row.pubkey, loadedRelay);
      setListGen((g) => g + 1);
    } catch (e) {
      const msg = adminErrorMessage(e);
      // 409 = no active ban — treat as a soft success (already gone).
      if (msg.includes("no active ban") || msg.includes("conflict")) {
        setListGen((g) => g + 1);
      } else {
        setLiftError(msg);
      }
    } finally {
      setWorkingPubkey(null);
    }
  };

  const handleConfirmLiftTimeout = async () => {
    const row = pendingLiftTimeout;
    if (!row) return;
    setPendingLiftTimeout(null);
    setLiftError(null);
    setWorkingPubkey(row.pubkey);
    try {
      await liftAdminTimeout(origin, row.pubkey, loadedRelay);
      setListGen((g) => g + 1);
    } catch (e) {
      const msg = adminErrorMessage(e);
      // 409 = no active timeout — treat as a soft success (already gone).
      if (msg.includes("no active timeout") || msg.includes("conflict")) {
        setListGen((g) => g + 1);
      } else {
        setLiftError(msg);
      }
    } finally {
      setWorkingPubkey(null);
    }
  };

  // Pages past the first, tied to the load generation so any reload (e.g.
  // after a lift) drops them and restarts from page one.
  const loadGen = generation + listGen;
  const [more, setMore] = useState<{
    gen: number;
    items: AdminMemberRestrictionDto[];
    nextCursor: string | null;
  } | null>(null);
  // Busy/error for the in-flight extra page, fenced by the same generation:
  // a page request that settles after a reload must not touch the new list.
  const [moreRequest, setMoreRequest] = useState<{
    gen: number;
    busy: boolean;
    error: string | null;
  } | null>(null);
  const loadGenRef = useRef(loadGen);
  useEffect(() => {
    loadGenRef.current = loadGen;
  }, [loadGen]);
  const extra = more?.gen === loadGen ? more : null;
  const moreStatus = moreRequest?.gen === loadGen ? moreRequest : null;
  const items =
    listState.status === "ok"
      ? [...listState.data.items, ...(extra?.items ?? [])]
      : [];
  const nextCursor =
    listState.status === "ok"
      ? extra
        ? extra.nextCursor
        : listState.data.nextCursor
      : null;

  const handleLoadMore = async () => {
    if (!nextCursor) return;
    const gen = loadGen;
    setMoreRequest({ gen, busy: true, error: null });
    try {
      const page = await listAdminRestrictions(origin, loadedRelay, nextCursor);
      if (loadGenRef.current !== gen) return;
      setMore({
        gen,
        items: [...(extra?.items ?? []), ...page.items],
        nextCursor: page.nextCursor,
      });
      setMoreRequest({ gen, busy: false, error: null });
    } catch (e) {
      if (loadGenRef.current !== gen) return;
      setMoreRequest({ gen, busy: false, error: adminErrorMessage(e) });
    }
  };

  return (
    <div className="space-y-2" data-testid="restrictions-section">
      {/* Lift-ban confirmation dialog */}
      <AlertDialog
        open={pendingLiftBan !== null}
        onOpenChange={(open) => {
          if (!open) setPendingLiftBan(null);
        }}
      >
        <AlertDialogContent data-testid="restrictions-lift-ban-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Lift ban?</AlertDialogTitle>
            <AlertDialogDescription>
              This will remove the active ban for{" "}
              <span className="font-mono">
                {pendingLiftBan ? truncatePubkey(pendingLiftBan.pubkey) : ""}
              </span>
              . They will be able to post again.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="restrictions-lift-ban-cancel">
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              asChild
              data-testid="restrictions-lift-ban-confirm"
            >
              <Button
                onClick={() => void handleConfirmLiftBan()}
                variant="default"
              >
                Lift ban
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Lift-timeout confirmation dialog */}
      <AlertDialog
        open={pendingLiftTimeout !== null}
        onOpenChange={(open) => {
          if (!open) setPendingLiftTimeout(null);
        }}
      >
        <AlertDialogContent data-testid="restrictions-lift-timeout-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Clear timeout?</AlertDialogTitle>
            <AlertDialogDescription>
              This will clear the active timeout for{" "}
              <span className="font-mono">
                {pendingLiftTimeout
                  ? truncatePubkey(pendingLiftTimeout.pubkey)
                  : ""}
              </span>
              . They will be able to post again.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="restrictions-lift-timeout-cancel">
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              asChild
              data-testid="restrictions-lift-timeout-confirm"
            >
              <Button
                onClick={() => void handleConfirmLiftTimeout()}
                variant="default"
              >
                Clear timeout
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <p className="text-xs font-medium text-muted-foreground">
        Active restrictions
      </p>

      {listState.status === "loading" && <LoadingSpinner />}
      {listState.status === "error" && (
        <ErrorMessage message={listState.message} />
      )}
      {liftError && <ErrorMessage message={liftError} />}
      {listState.status === "ok" && items.length === 0 && (
        <p
          className="text-sm text-muted-foreground"
          data-testid="restrictions-empty"
        >
          No active bans or timeouts.
        </p>
      )}
      {listState.status === "ok" && items.length > 0 && (
        <ul className="space-y-1">
          {items.map((row) => {
            const isWorking = workingPubkey === row.pubkey;
            const now = new Date();
            const isBanned =
              row.banned &&
              (row.banExpiresAt === null || new Date(row.banExpiresAt) > now);
            const isTimedOut =
              row.mutedUntil !== null && new Date(row.mutedUntil) > now;
            return (
              <li
                className="flex items-center gap-2 rounded-md border border-border/60 px-3 py-2"
                data-testid={`restriction-row-${row.pubkey}`}
                key={row.pubkey}
              >
                <div className="flex-1 min-w-0">
                  <p className="text-xs font-mono truncate">
                    {truncatePubkey(row.pubkey)}
                  </p>
                  <div className="mt-0.5">
                    <RestrictionTypeBadge record={row} />
                  </div>
                </div>
                {isBanned && (
                  <Button
                    data-testid={`restrictions-lift-ban-btn-${row.pubkey}`}
                    disabled={isWorking}
                    onClick={() => setPendingLiftBan(row)}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    {isWorking ? (
                      <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      "Lift ban"
                    )}
                  </Button>
                )}
                {isTimedOut && (
                  <Button
                    data-testid={`restrictions-lift-timeout-btn-${row.pubkey}`}
                    disabled={isWorking}
                    onClick={() => setPendingLiftTimeout(row)}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    {isWorking ? (
                      <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      "Clear timeout"
                    )}
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {moreStatus?.error && <ErrorMessage message={moreStatus.error} />}
      {nextCursor && (
        <Button
          data-testid="restrictions-load-more"
          disabled={moreStatus?.busy ?? false}
          onClick={() => void handleLoadMore()}
          size="sm"
          type="button"
          variant="outline"
        >
          {moreStatus?.busy ? (
            <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
          ) : (
            "Load more"
          )}
        </Button>
      )}
    </div>
  );
}

// ── Staffing tab ──────────────────────────────────────────────────────────

export function StaffingTab({
  origin,
  pubkey,
  generation,
  canMutate,
  onSelfMutation,
}: {
  origin: string;
  pubkey: string;
  generation: number;
  /**
   * When false (disabled-auth probe), all write affordances are hidden.
   * The operator list is still readable; only add/remove/edit controls are absent.
   */
  canMutate: boolean;
  /**
   * Called after a successful mutation that modified the current principal's
   * own operator row (role change or removal of self). The parent re-probes
   * the admin origin so the displayed role and visible tabs reflect the new
   * server state.
   */
  onSelfMutation?: () => void;
}) {
  const { activeCommunity } = useCommunities();
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
      // Surface the relay's parsed error message — a 409 may indicate either
      // a config-backed key (immutable) or a last-operator conflict, both of
      // which the relay communicates with distinct messages.
      setAddError(adminErrorMessage(e));
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
      // Self-demotion: re-probe so the parent updates role badge and tab
      // visibility to reflect the new server state.
      if (op.pubkey === pubkey) {
        onSelfMutation?.();
      }
    } catch (e) {
      setActionError(adminErrorMessage(e));
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
      // Self-removal: re-probe so the parent updates role badge and tab
      // visibility to reflect the new server state.
      if (op.pubkey === pubkey) {
        onSelfMutation?.();
      }
    } catch (e) {
      setActionError(adminErrorMessage(e));
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

      {/* Restrictions section — active bans and timeouts for the current community */}
      <div className="mt-6 rounded-md border border-border/60 px-3 py-2.5">
        <RestrictionsSection
          origin={origin}
          relayKey={activeCommunity?.relayUrl ?? ""}
          generation={generation}
        />
      </div>
    </div>
  );
}
