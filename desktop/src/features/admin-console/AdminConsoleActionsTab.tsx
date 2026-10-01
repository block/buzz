/**
 * Actions tab — ban, time out, or delete without a report.
 *
 * Review freezes the whole intent (origin, relay, signer, community, verb,
 * target, reason, duration, requestId). Confirm and every retry resend that
 * frozen intent, so the relay dedupes on the same requestId. Review and
 * Confirm share one in-flight lock, so a late Review can never replace an
 * intent that is frozen or being sent. The native layer
 * mints a fresh NIP-98 signature per attempt and refuses the send if the
 * active relay or signer moved. The panel remounts this tab on an identity or
 * origin change, which drops any frozen intent.
 *
 * The community defaults to the active relay's host (read-only, with a
 * Change escape hatch), and members are picked by name on that relay or by a
 * pasted npub/hex key anywhere.
 */

import { useDeferredValue, useRef, useState } from "react";
import { toast } from "sonner";
import { classifyKeyImportInput } from "@/features/onboarding/lib/keyImportInput";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  useUserProfileQuery,
  useUserSearchQuery,
} from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { SelectedRecipientChip } from "@/features/profile/ui/SelectedRecipientChip";
import { getRelayWsUrl } from "@/shared/api/tauri";
import type { UserSearchResult } from "@/shared/api/types";
import { parsePubkeyInput } from "@/shared/lib/nostrUtils";
import { truncateNpub } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { PubKey } from "@/shared/ui/PubKey";
import {
  directAdminAction,
  type AdminDirectAction,
  type AdminDirectIntent,
  communityHostFromRelayUrl,
  normalizeCommunityHost,
} from "./api";
import {
  adminErrorCode,
  adminErrorMessage,
  adminMutationBodyComplete,
  adminMutationNotSent,
  adminMutationRelayStatus,
  preserveRequestIdOnError,
  useAsyncLoad,
} from "./AdminConsolePanelHelpers";
import { reasonAudienceCopy } from "./AdminConsoleReportsTab";

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
      return "This request id was already used for a different action. Review again to send it with a new id.";
    case null: {
      // A relay from before direct actions answers these routes with an empty
      // 404/405; any other uncoded body keeps its own text.
      const status = adminMutationRelayStatus(e);
      if (
        (status === 404 || status === 405) &&
        adminMutationBodyComplete(e) &&
        adminErrorMessage(e).trim() === "admin API error:"
      ) {
        return "This relay doesn't support direct actions yet.";
      }
      return adminErrorMessage(e);
    }
    default:
      return adminErrorMessage(e);
  }
}

/** An `nsec` or NIP-49 backup, in either case. */
function isSecretKey(input: string): boolean {
  return classifyKeyImportInput(input.toLowerCase()) !== "unknown";
}

function memberLabel(user: UserSearchResult): string {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncateNpub(user.pubkey)
  );
}

/** Client-side checks; the relay re-validates everything. */
function validate(
  action: AdminDirectAction,
  host: string,
  target: string,
  member: UserSearchResult | null,
  secs: string,
): string | null {
  if (!host.trim()) return "Enter the community host.";
  if (action === "delete" && !HEX64.test(target.trim())) {
    return "Event id must be 64 lowercase hex characters.";
  }
  if (action !== "delete" && !member) {
    return "Choose a member: search by name, or paste an npub or hex key.";
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
  const { activeCommunity } = useCommunities();
  /** Operator-typed host; null while following the active community. */
  const [hostOverride, setHostOverride] = useState<string | null>(null);
  const [target, setTarget] = useState("");
  const [member, setMember] = useState<UserSearchResult | null>(null);
  /** Display-only name for the frozen target; never sent. */
  const [frozenName, setFrozenName] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [secs, setSecs] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [frozen, setFrozen] = useState<AdminDirectIntent | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const inFlight = useRef(false);

  // Host of the active relay: the default target community. Reloads when the
  // desktop switches communities.
  const activeHostState = useAsyncLoad(
    async () => communityHostFromRelayUrl(await getRelayWsUrl()),
    [activeCommunity?.relayUrl ?? ""],
    0,
  );
  const activeHost =
    activeHostState.status === "ok" ? activeHostState.data : null;

  const editingHost = hostOverride !== null || !activeHost;
  const host = hostOverride ?? activeHost ?? "";
  // Name search queries the active relay, so it only finds members there.
  const normalizedHost = normalizeCommunityHost(host);
  const onActiveCommunity =
    activeHost !== null && normalizedHost === activeHost;
  // Profiles are per community: a new host drops the picked member.
  const editHost = (next: string) => {
    if (normalizeCommunityHost(next) !== normalizedHost) setMember(null);
    setHostOverride(next);
  };

  const handleReview = async () => {
    if (frozen || inFlight.current) return;
    const invalid = validate(action, host, target, member, secs);
    setError(invalid);
    if (invalid) return;
    inFlight.current = true;
    setSubmitting(true);
    try {
      const expectedRelay = await getRelayWsUrl();
      // Following the active community: derive the host from the very relay
      // URL being frozen, so the two can never disagree.
      const communityHost =
        hostOverride?.trim() ?? communityHostFromRelayUrl(expectedRelay);
      if (!communityHost) {
        setError("Enter the community host.");
        return;
      }
      const isMember = action !== "delete" && member;
      setFrozenName(
        isMember && (member.displayName || member.nip05Handle)
          ? memberLabel(member)
          : null,
      );
      setFrozen({
        origin,
        expectedRelay,
        expectedPubkey: pubkey,
        communityHost,
        action,
        target: isMember ? member.pubkey : target.trim(),
        requestId: crypto.randomUUID(),
        reason: reason.trim() || undefined,
        expirationSecs: action === "timeout" ? Number(secs) : undefined,
      });
    } catch (e) {
      setError(adminErrorMessage(e));
    } finally {
      inFlight.current = false;
      setSubmitting(false);
    }
  };

  const handleConfirm = async () => {
    if (!frozen || inFlight.current) return;
    inFlight.current = true;
    setSubmitting(true);
    setError(null);
    setPending(false);
    try {
      const result = await directAdminAction(frozen);
      if (result.state === "pending") {
        setPending(true);
      } else {
        toast.success(`${ACTION_LABELS[frozen.action]}: done`);
        setFrozen(null);
        setTarget("");
        setMember(null);
        setReason("");
      }
    } catch (e) {
      // Keep the frozen intent (same requestId) unless the relay definitively
      // rejected it before committing. A request-id conflict is final for
      // this id, and a refusal before sending (bad host, relay or signer
      // moved) repeats on every resend, so neither can ever succeed.
      if (
        !preserveRequestIdOnError(e) ||
        adminMutationNotSent(e) ||
        adminErrorCode(e) === "request_id_conflict"
      ) {
        setFrozen(null);
      }
      setError(directErrorMessage(e));
    } finally {
      inFlight.current = false;
      setSubmitting(false);
    }
  };

  const locked = frozen !== null || submitting || !canMutate;
  // A pasted key has no name yet; look it up where search would find it.
  const confirmProfile = useUserProfileQuery(
    frozen &&
      frozen.action !== "delete" &&
      !frozenName &&
      frozen.communityHost === activeHost
      ? frozen.target
      : undefined,
  );
  const confirmName =
    frozenName ?? confirmProfile.data?.displayName?.trim() ?? null;
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
      {editingHost ? (
        input("host", host, editHost, "Community host (e.g. team.example.com)")
      ) : (
        <p
          className="flex items-center gap-2 text-xs"
          data-testid="direct-host"
        >
          <span>
            Community: <code>{host}</code>
          </span>
          <Button
            className="h-auto p-0 text-xs"
            data-testid="direct-host-change"
            disabled={locked}
            onClick={() => setHostOverride(host)}
            size="sm"
            type="button"
            variant="link"
          >
            Change
          </Button>
        </p>
      )}
      {action === "delete" ? (
        input("target", target, setTarget, "Event id (hex)", "font-mono")
      ) : (
        <MemberPicker
          disabled={locked}
          key={normalizedHost}
          member={member}
          onChange={setMember}
          searchEnabled={onActiveCommunity}
        />
      )}
      {action === "timeout" &&
        input("duration", secs, setSecs, "Duration (seconds)", "", "number")}
      {input("reason", reason, setReason, "Reason (optional)")}
      <p
        className="text-xs text-muted-foreground"
        data-testid="direct-reason-audience"
      >
        {reasonAudienceCopy(frozen?.action ?? action)}
      </p>
      {pending && (
        <p
          className="text-xs text-muted-foreground"
          data-testid="direct-pending"
        >
          Accepted; the relay is still applying it. Retry to check.
        </p>
      )}
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
            {ACTION_LABELS[frozen.action]}{" "}
            {frozen.action === "delete" ? (
              <code>{frozen.target}</code>
            ) : (
              <span data-testid="direct-confirm-member">
                {confirmName ? `${confirmName} ` : ""}
                <code>
                  {confirmName
                    ? `(${truncateNpub(frozen.target)})`
                    : truncateNpub(frozen.target)}
                </code>
              </span>
            )}{" "}
            in <code>{frozen.communityHost}</code>
            {frozen.expirationSecs ? ` for ${frozen.expirationSecs}s` : ""}?
          </p>
          {frozen.action !== "delete" && (
            <PubKey
              pubkey={frozen.target}
              testId="direct-confirm-npub"
              variant="full"
            />
          )}
          <p data-testid="direct-confirm-reason">
            Reason: {frozen.reason ?? "(none)"}
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
              {error || pending ? "Retry" : "Confirm"}
            </Button>
            <Button
              data-testid="direct-discard-btn"
              disabled={submitting}
              onClick={() => {
                setFrozen(null);
                setError(null);
                setPending(false);
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
          disabled={!canMutate || submitting}
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

/**
 * Pick a ban/timeout target: search names on the active relay, or paste an
 * npub or hex key. Mirrors the Add Member dialog's picker.
 */
function MemberPicker({
  disabled,
  member,
  onChange,
  searchEnabled,
}: {
  disabled: boolean;
  member: UserSearchResult | null;
  onChange: (member: UserSearchResult | null) => void;
  /** Off when the target community is not the active relay's. */
  searchEnabled: boolean;
}) {
  const [query, setQuery] = useState("");
  const [inspecting, setInspecting] = useState(false);
  const deferred = useDeferredValue(query.trim());
  const parsed = parsePubkeyInput(deferred);
  // A pasted secret key must never leave the device as a search query.
  const secret = isSecretKey(query) || isSecretKey(deferred);
  const search = useUserSearchQuery(deferred, {
    enabled: searchEnabled && !secret && deferred.length > 0 && parsed === null,
    limit: 8,
  });
  const results: UserSearchResult[] = parsed
    ? [
        {
          pubkey: parsed,
          displayName: null,
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
          isAgent: false,
        },
      ]
    : searchEnabled && !secret
      ? (search.data ?? [])
      : [];

  const hint = !searchEnabled && (
    <p
      className="text-xs text-muted-foreground"
      data-testid="direct-member-search-hint"
    >
      Name search only works in the community you're connected to.
    </p>
  );
  if (member) {
    return (
      <div className="space-y-1" data-testid="direct-member-selected">
        <SelectedRecipientChip
          disabled={disabled}
          inspectionOpen={inspecting}
          label={memberLabel(member)}
          onInspectionOpenChange={setInspecting}
          onRemove={() => onChange(null)}
          poofOnRemove={false}
          testIds={{
            chip: "direct-member-remove",
            name: "direct-member-inspect",
            pubkey: "direct-member-npub",
          }}
          user={member}
        />
        {hint}
      </div>
    );
  }
  return (
    <div className="space-y-1">
      <input
        className="w-full rounded-md border border-border/60 bg-background px-2 py-1 text-xs"
        data-testid="direct-member-input"
        disabled={disabled}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={
          searchEnabled
            ? "Search by name, or paste an npub or hex key"
            : "npub or hex key"
        }
        value={query}
      />
      {secret && (
        <p
          className="text-xs text-destructive"
          data-testid="direct-member-secret"
        >
          That's a secret key. Never paste it here.
        </p>
      )}
      {hint}
      {results.length > 0 && (
        <div className="rounded-md border border-border/60" role="listbox">
          {results.map((user) => (
            <button
              className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-xs hover:bg-muted/50"
              data-testid={`direct-member-result-${user.pubkey}`}
              disabled={disabled}
              key={user.pubkey}
              onClick={() => {
                onChange(user);
                setQuery("");
              }}
              role="option"
              type="button"
            >
              <ProfileAvatar
                avatarUrl={user.avatarUrl}
                className="h-6 w-6 text-2xs shadow-none"
                iconClassName="h-3 w-3"
                label={memberLabel(user)}
                shape={user.isAgent ? "squircle" : "circle"}
              />
              <span className="min-w-0 flex-1 truncate">
                {memberLabel(user)}
              </span>
              {parsed ? (
                <span className="text-muted-foreground">public key</span>
              ) : (
                <PubKey
                  className="text-muted-foreground"
                  interactive={false}
                  pubkey={user.pubkey}
                />
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
