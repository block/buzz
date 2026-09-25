/**
 * Actions tab — ban, time out, or delete without a report.
 *
 * Review freezes the whole intent (origin, relay, signer, community, verb,
 * target, reason, duration, requestId). Confirm and every retry resend that
 * frozen intent, so the relay dedupes on the same requestId; the native layer
 * mints a fresh NIP-98 signature per attempt and refuses the send if the
 * active relay or signer moved. The panel remounts this tab on an identity or
 * origin change, which drops any frozen intent.
 */

import { useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/shared/ui/button";
import { getRelayWsUrl } from "@/shared/api/tauri";
import {
  directAdminAction,
  type AdminDirectAction,
  type AdminDirectIntent,
} from "./api";
import {
  adminErrorCode,
  adminErrorMessage,
  preserveRequestIdOnError,
} from "./AdminConsolePanelHelpers";

const HEX64 = /^[0-9a-f]{64}$/;

const ACTION_LABELS: Record<AdminDirectAction, string> = {
  ban: "Ban member",
  timeout: "Time out member",
  delete: "Delete message",
};

function directErrorMessage(e: unknown): string {
  switch (adminErrorCode(e)) {
    case "target_is_staff":
      return "Relay staff can't be banned or timed out. Remove their staff role first.";
    case "request_id_conflict":
      return "This request id was already used for a different action. Discard and review again.";
    default:
      return adminErrorMessage(e);
  }
}

/** Client-side checks; the relay re-validates everything. */
function validate(
  action: AdminDirectAction,
  host: string,
  target: string,
  secs: string,
): string | null {
  if (!host.trim()) return "Enter the community host.";
  if (!HEX64.test(target.trim())) {
    return action === "delete"
      ? "Event id must be 64 lowercase hex characters."
      : "Member pubkey must be 64 lowercase hex characters.";
  }
  if (
    action === "timeout" &&
    !(Number.isInteger(Number(secs)) && Number(secs) > 0)
  ) {
    return "Duration must be a whole number of seconds above zero.";
  }
  return null;
}

export function ActionsTab({
  canMutate,
  origin,
  pubkey,
}: {
  canMutate: boolean;
  origin: string;
  /** Active signer; frozen into the intent at review. */
  pubkey: string;
}) {
  const [action, setAction] = useState<AdminDirectAction>("ban");
  const [host, setHost] = useState("");
  const [target, setTarget] = useState("");
  const [reason, setReason] = useState("");
  const [secs, setSecs] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [frozen, setFrozen] = useState<AdminDirectIntent | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const inFlight = useRef(false);

  const handleReview = async () => {
    const invalid = validate(action, host, target, secs);
    setError(invalid);
    if (invalid) return;
    setFrozen({
      origin,
      expectedRelay: await getRelayWsUrl(),
      expectedPubkey: pubkey,
      communityHost: host.trim(),
      action,
      target: target.trim(),
      requestId: crypto.randomUUID(),
      reason: reason.trim() || undefined,
      expirationSecs: action === "timeout" ? Number(secs) : undefined,
    });
  };

  const handleConfirm = async () => {
    if (!frozen || inFlight.current) return;
    inFlight.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const result = await directAdminAction(frozen);
      if (result.state === "pending") {
        setError("Accepted; the relay is still applying it. Retry to check.");
      } else {
        toast.success(`${ACTION_LABELS[frozen.action]}: done`);
        setFrozen(null);
        setTarget("");
        setReason("");
      }
    } catch (e) {
      // Keep the frozen intent (same requestId) unless the relay definitively
      // rejected it before committing.
      if (!preserveRequestIdOnError(e)) setFrozen(null);
      setError(directErrorMessage(e));
    } finally {
      inFlight.current = false;
      setSubmitting(false);
    }
  };

  const locked = frozen !== null || !canMutate;
  const targetHint =
    action === "delete" ? "Event id (hex)" : "Member pubkey (hex)";
  const input = (
    name: string,
    value: string,
    set: (v: string) => void,
    placeholder: string,
    extra = "",
    type = "text",
  ) => (
    <input
      className={`w-full rounded-md border border-border/60 bg-background px-2 py-1 text-xs ${extra}`}
      data-testid={`direct-${name}-input`}
      disabled={locked}
      onChange={(e) => set(e.target.value)}
      placeholder={placeholder}
      type={type}
      value={value}
    />
  );

  return (
    <div className="space-y-3" data-testid="actions-tab">
      <div className="flex gap-1.5">
        {(Object.keys(ACTION_LABELS) as AdminDirectAction[]).map((a) => (
          <Button
            data-testid={`direct-action-${a}`}
            disabled={locked}
            key={a}
            onClick={() => setAction(a)}
            size="sm"
            type="button"
            variant={a === action ? "default" : "outline"}
          >
            {ACTION_LABELS[a]}
          </Button>
        ))}
      </div>
      {input("host", host, setHost, "Community host (e.g. team.example.com)")}
      {input("target", target, setTarget, targetHint, "font-mono")}
      {action === "timeout" &&
        input("duration", secs, setSecs, "Duration (seconds)", "", "number")}
      {input("reason", reason, setReason, "Reason (optional)")}
      {error && (
        <p className="text-xs text-destructive" data-testid="direct-error">
          {error}
        </p>
      )}
      {frozen ? (
        <div
          className="space-y-2 rounded-md border border-border/60 px-3 py-2 text-xs"
          data-testid="direct-confirm"
        >
          <p>
            {ACTION_LABELS[frozen.action]} <code>{frozen.target}</code> in{" "}
            <code>{frozen.communityHost}</code>
            {frozen.expirationSecs ? ` for ${frozen.expirationSecs}s` : ""}?
          </p>
          <div className="flex gap-1.5">
            <Button
              data-testid="direct-confirm-btn"
              disabled={!canMutate || submitting}
              onClick={() => void handleConfirm()}
              size="sm"
              type="button"
              variant="destructive"
            >
              {error ? "Retry" : "Confirm"}
            </Button>
            <Button
              data-testid="direct-discard-btn"
              disabled={submitting}
              onClick={() => {
                setFrozen(null);
                setError(null);
              }}
              size="sm"
              type="button"
              variant="ghost"
            >
              Discard
            </Button>
          </div>
        </div>
      ) : (
        <Button
          data-testid="direct-review-btn"
          disabled={!canMutate}
          onClick={() => void handleReview()}
          size="sm"
          type="button"
        >
          Review
        </Button>
      )}
    </div>
  );
}
