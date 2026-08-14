/**
 * Staffing tab — Operator-only UI for managing relay_operators rows.
 *
 * Source badges distinguish config-backed entries (immutable via API) from
 * DB-managed entries (can be added/removed). 409 conflicts from the server
 * (config-backed key modification attempts) are surfaced with a clear message.
 */

import { useState } from "react";
import { LoaderCircle, Trash2 } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
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
}: {
  origin: string;
  pubkey: string;
  generation: number;
}) {
  const [listGen, setListGen] = useState(0);
  const [addPubkey, setAddPubkey] = useState("");
  const [addRole, setAddRole] = useState<"operator" | "moderator">("moderator");
  const [isAdding, setIsAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [workingPubkey, setWorkingPubkey] = useState<string | null>(null);

  const listState: AsyncState<AdminOperatorDto[]> = useAsyncLoad(
    () => listAdminOperators(origin),
    [origin, pubkey],
    generation + listGen,
  );

  const handleAdd = async () => {
    const trimmed = addPubkey.trim().toLowerCase();
    if (!trimmed) return;
    setAddError(null);
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

  const handleRemove = async (opPubkey: string) => {
    setActionError(null);
    setWorkingPubkey(opPubkey);
    try {
      await deleteAdminOperator(origin, opPubkey);
      setListGen((g) => g + 1);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setActionError(
        msg.includes("409")
          ? `Cannot remove ${opPubkey.slice(0, 16)}…: config-backed key.`
          : msg,
      );
    } finally {
      setWorkingPubkey(null);
    }
  };

  return (
    <div className="space-y-4" data-testid="staffing-tab">
      {/* Add operator form */}
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
            disabled={isAdding || !addPubkey.trim()}
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
            return (
              <li
                className="flex items-center gap-2 rounded-md border border-border/60 px-3 py-2"
                data-testid={`staffing-row-${op.pubkey}`}
                key={op.pubkey}
              >
                <div className="flex-1 min-w-0">
                  <p className="text-xs font-mono truncate">{op.pubkey}</p>
                  <div className="flex flex-wrap gap-1 mt-0.5">
                    <Badge variant="outline">{op.effectiveRole}</Badge>
                    {op.sources.map((s) => (
                      <SourceBadge key={s} source={s} />
                    ))}
                  </div>
                </div>
                <Button
                  aria-label={`Remove ${op.pubkey}`}
                  data-testid={`staffing-remove-btn-${op.pubkey}`}
                  disabled={isConfigBacked || workingPubkey === op.pubkey}
                  onClick={() => void handleRemove(op.pubkey)}
                  size="icon-xs"
                  title={
                    isConfigBacked
                      ? "Config-backed — cannot be removed via API"
                      : "Remove operator"
                  }
                  type="button"
                  variant="ghost"
                >
                  {workingPubkey === op.pubkey ? (
                    <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="h-3.5 w-3.5" />
                  )}
                </Button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
