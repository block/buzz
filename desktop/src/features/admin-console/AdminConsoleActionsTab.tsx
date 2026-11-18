/**
 * Direct actions — ban, time out, or delete without a report — in one
 * community.
 *
 * `DirectActionsProvider` owns the controller at panel level, so a frozen,
 * pending or ambiguous intent (and its requestId) survives tab and community
 * page changes; only an explicit Discard drops it. Review freezes the whole
 * intent (origin, relay, signer, community, verb, target, reason, duration,
 * requestId). Confirm and every retry resend that frozen intent, so the relay
 * dedupes on the same requestId. Review and Confirm share one in-flight lock.
 * The native layer mints a fresh NIP-98 signature per attempt and refuses the
 * send if the active relay or signer moved. The panel remounts the provider
 * on an identity or origin change.
 *
 * `ActionsSection` is the form for one community page's host. Review needs
 * the target's `/members/{pk}` state (staff blocks it) or the event preview,
 * each fenced by (origin, signer, community, target, generation) so a late
 * answer for another community can never unlock this one.
 */

import {
  createContext,
  type ReactNode,
  useContext,
  useDeferredValue,
  useEffect,
  useRef,
  useState,
} from "react";
import { toast } from "sonner";
import { containsSecretKey } from "@/features/onboarding/lib/keyImportInput";
import {
  MemberSearchResult,
  memberSearchLabel,
  useMemberCandidates,
} from "@/features/profile/ui/MemberSearchResult";
import { getRelayWsUrl } from "@/shared/api/tauri";
import type { UserSearchResult } from "@/shared/api/types";
import { parseEventIdInput } from "@/shared/lib/nostrUtils";
import { truncateNpub } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { PubKey } from "@/shared/ui/PubKey";
import {
  directAdminAction,
  getAdminEvent,
  getAdminMember,
  searchAdminMembers,
  type AdminDirectAction,
  type AdminDirectIntent,
  type AdminEventPreviewDto,
  type AdminMemberDetailDto,
} from "./api";
import {
  CommunityBadge,
  type CommunityRef,
  NotConnectedWarning,
} from "./AdminConsoleCommunityBadge";
import {
  adminErrorCode,
  adminErrorMessage,
  adminMutationNotSent,
  adminReadErrorCode,
  adminRouteUnsupported,
  type AsyncState,
  formatAbsoluteTimestamp,
  formatTimestamp,
  preserveRequestIdOnError,
  UNSUPPORTED_BROWSING,
} from "./AdminConsolePanelHelpers";
import { reasonAudienceCopy } from "./AdminConsoleReportsTab";

const ACTION_LABELS: Record<AdminDirectAction, string> = {
  ban: "Ban member",
  timeout: "Time out member",
  delete: "Delete message",
};

const STAFF_COPY =
  "Relay staff can't be banned or timed out. Remove their staff role first.";

function directErrorMessage(e: unknown): string {
  switch (adminErrorCode(e)) {
    case "target_is_staff":
      return STAFF_COPY;
    case "request_id_conflict":
      return "This request id was already used for a different action. Review again to send it with a new id.";
    default:
      return adminRouteUnsupported(e)
        ? "This relay doesn't support direct actions yet."
        : adminErrorMessage(e);
  }
}

/** Why a read failed, in words; never "not found" unless the relay said so. */
function readErrorMessage(e: unknown, notFound: string): string {
  if (adminRouteUnsupported(e)) return UNSUPPORTED_BROWSING;
  if (adminReadErrorCode(e) === "event_not_found") return notFound;
  return adminErrorMessage(e);
}

/**
 * Load `load()` for `key`; a result is shown only while its key is current.
 * `null` key means nothing to load.
 */
export function useFencedLoad<T>(
  key: string | null,
  load: () => Promise<T>,
): AsyncState<T> {
  const [result, setResult] = useState<{
    key: string;
    state: AsyncState<T>;
  } | null>(null);
  const loadRef = useRef(load);
  loadRef.current = load;
  useEffect(() => {
    if (key === null) return;
    let active = true;
    loadRef.current().then(
      (data) => active && setResult({ key, state: { status: "ok", data } }),
      (error: unknown) =>
        active &&
        setResult({
          key,
          state: { status: "error", message: adminErrorMessage(error), error },
        }),
    );
    return () => {
      active = false;
    };
  }, [key]);
  if (key === null) return { status: "idle" };
  return result?.key === key ? result.state : { status: "loading" };
}

// ── Panel-level controller ────────────────────────────────────────────────

type Draft = {
  host: string;
  action: AdminDirectAction;
  target: string;
  member: UserSearchResult | null;
  reason: string;
  secs: string;
};

const emptyDraft = (host: string): Draft => ({
  host,
  action: "ban",
  target: "",
  member: null,
  reason: "",
  secs: "",
});

/** The frozen intent plus what the confirm step shows about its target. */
type Frozen = {
  intent: AdminDirectIntent;
  community: CommunityRef;
  name: string | null;
  member: AdminMemberDetailDto | null;
  preview: AdminEventPreviewDto | null;
};

type DirectActions = {
  canMutate: boolean;
  origin: string;
  pubkey: string;
  generation: number;
  draftFor: (host: string) => Draft;
  setDraft: (draft: Draft) => void;
  frozen: Frozen | null;
  pending: boolean;
  error: string | null;
  submitting: boolean;
  review: (
    draft: Draft,
    target: string,
    lookup: Omit<Frozen, "intent" | "name">,
  ) => Promise<void>;
  confirm: () => Promise<void>;
  discard: () => void;
  /** Host whose Actions section is on screen, for the hidden-failure toast. */
  shownHost: { current: string | null };
};

const DirectActionsContext = createContext<DirectActions | null>(null);

export function useDirectActions(): DirectActions {
  const ctx = useContext(DirectActionsContext);
  if (!ctx) throw new Error("useDirectActions outside DirectActionsProvider");
  return ctx;
}

export function DirectActionsProvider({
  canMutate,
  origin,
  pubkey,
  generation,
  children,
}: {
  canMutate: boolean;
  origin: string;
  /** Active signer; frozen into the intent at review. */
  pubkey: string;
  generation: number;
  children: ReactNode;
}) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [frozen, setFrozen] = useState<Frozen | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const inFlight = useRef(false);
  const shownHost = useRef<string | null>(null);

  const locked = async (run: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setSubmitting(true);
    try {
      await run();
    } finally {
      inFlight.current = false;
      setSubmitting(false);
    }
  };

  const review: DirectActions["review"] = (d, target, lookup) =>
    locked(async () => {
      if (frozen) return;
      setError(null);
      try {
        const expectedRelay = await getRelayWsUrl();
        const m = d.action !== "delete" ? d.member : null;
        setFrozen({
          intent: {
            origin,
            expectedRelay,
            expectedPubkey: pubkey,
            communityHost: d.host,
            action: d.action,
            target,
            requestId: crypto.randomUUID(),
            reason: d.reason.trim() || undefined,
            expirationSecs: d.action === "timeout" ? Number(d.secs) : undefined,
          },
          name:
            m && (m.displayName || m.nip05Handle) ? memberSearchLabel(m) : null,
          ...lookup,
        });
      } catch (e) {
        setError(adminErrorMessage(e));
      }
    });

  const confirm = () =>
    locked(async () => {
      if (!frozen) return;
      const { intent } = frozen;
      setError(null);
      setPending(false);
      try {
        const result = await directAdminAction(intent);
        if (result.state === "pending") {
          setPending(true);
        } else {
          toast.success(`${ACTION_LABELS[intent.action]}: done`);
          setFrozen(null);
          setDraft(emptyDraft(intent.communityHost));
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
        const message = directErrorMessage(e);
        setError(message);
        if (shownHost.current !== intent.communityHost) {
          toast.error(
            `${ACTION_LABELS[intent.action]} in ${intent.communityHost} failed: ${message}`,
          );
        }
      }
    });

  const value: DirectActions = {
    canMutate,
    origin,
    pubkey,
    generation,
    draftFor: (host) => (draft?.host === host ? draft : emptyDraft(host)),
    setDraft,
    frozen,
    pending,
    error,
    submitting,
    review,
    confirm,
    discard: () => {
      setFrozen(null);
      setError(null);
      setPending(false);
    },
    shownHost,
  };
  return (
    <DirectActionsContext.Provider value={value}>
      {children}
    </DirectActionsContext.Provider>
  );
}

// ── Actions section ───────────────────────────────────────────────────────

/** Client-side checks; the relay re-validates everything. */
function validate(draft: Draft, eventId: string | null): string | null {
  if (draft.action === "delete" && !eventId) {
    return "Enter a 64-hex event id, or a note1… or nevent1… link.";
  }
  if (draft.action !== "delete" && !draft.member) {
    return "Choose a member: search by name, or paste an npub or hex key.";
  }
  const secs = Number(draft.secs);
  if (draft.action === "timeout" && !(Number.isInteger(secs) && secs > 0)) {
    return "Duration must be a whole number of seconds above zero.";
  }
  return null;
}

function MemberState({ member }: { member: AdminMemberDetailDto }) {
  const facts = [
    member.role === null
      ? "Not on the community roster"
      : `Role: ${member.role}`,
    member.banned ? "Currently banned" : null,
    member.mutedUntil
      ? `Timed out until ${formatAbsoluteTimestamp(member.mutedUntil)}`
      : null,
    member.isStaff ? "Relay staff" : null,
  ].filter(Boolean);
  return (
    <p
      className="text-xs text-muted-foreground"
      data-testid="direct-member-state"
    >
      {facts.join(" · ")}
    </p>
  );
}

function EventPreview({ event }: { event: AdminEventPreviewDto }) {
  return (
    <div
      className="space-y-1 rounded-md border border-border/60 px-2 py-1.5 text-xs"
      data-testid="direct-event-preview"
    >
      <PubKey pubkey={event.authorPubkey} variant="full" />
      <p className="whitespace-pre-wrap break-words">{event.content}</p>
      {event.deletedAt && (
        <p className="text-muted-foreground">
          Already deleted {formatTimestamp(event.deletedAt)}.
        </p>
      )}
    </div>
  );
}

export function ActionsSection({ community }: { community: CommunityRef }) {
  const communityHost = community.host;
  const c = useDirectActions();
  const { frozen } = c;
  const draft = c.draftFor(communityHost);
  const set = (patch: Partial<Draft>) => c.setDraft({ ...draft, ...patch });
  const [invalid, setInvalid] = useState<string | null>(null);

  useEffect(() => {
    c.shownHost.current = communityHost;
    return () => {
      c.shownHost.current = null;
    };
  }, [c.shownHost, communityHost]);

  const eventId =
    draft.action === "delete" ? parseEventIdInput(draft.target) : null;
  const memberPk =
    draft.action !== "delete" ? (draft.member?.pubkey ?? null) : null;
  const fence = `${c.origin}\n${c.pubkey}\n${communityHost}\n${c.generation}`;
  const memberState = useFencedLoad(
    memberPk && `${fence}\nmember\n${memberPk}`,
    () => getAdminMember(c.origin, communityHost, memberPk ?? ""),
  );
  const previewState = useFencedLoad(
    eventId && `${fence}\nevent\n${eventId}`,
    () => getAdminEvent(c.origin, communityHost, eventId ?? ""),
  );
  const lookup = draft.action === "delete" ? previewState : memberState;
  const lookupBlock =
    lookup.status === "error"
      ? readErrorMessage(lookup.error, "Not found in this community.")
      : memberState.status === "ok" &&
          draft.action !== "delete" &&
          memberState.data.isStaff
        ? STAFF_COPY
        : null;

  if (frozen && frozen.intent.communityHost !== communityHost) {
    return (
      <div className="space-y-2 text-xs" data-testid="direct-elsewhere">
        <p>
          Finish or discard the pending action in{" "}
          <code>{frozen.intent.communityHost}</code>.
        </p>
        <ConfirmStep readOnly />
      </div>
    );
  }

  const locked = frozen !== null || c.submitting || !c.canMutate;
  const handleReview = () => {
    const problem = validate(draft, eventId);
    setInvalid(problem);
    if (problem || lookup.status !== "ok" || lookupBlock) return;
    void c.review(draft, eventId ?? memberPk ?? "", {
      community,
      member: memberState.status === "ok" ? memberState.data : null,
      preview:
        draft.action === "delete" && previewState.status === "ok"
          ? previewState.data
          : null,
    });
  };
  const input = (
    name: keyof Draft,
    placeholder: string,
    extra = "",
    type = "text",
  ) => (
    <input
      className={`w-full rounded-md border border-border/60 bg-background px-2 py-1 text-xs ${extra}`}
      data-testid={`direct-${name === "secs" ? "duration" : name}-input`}
      disabled={locked}
      onChange={(e) => set({ [name]: e.target.value })}
      placeholder={placeholder}
      type={type}
      value={draft[name] as string}
    />
  );
  const error = invalid ?? c.error;

  return (
    <div className="space-y-3" data-testid="actions-tab">
      <div className="flex gap-1.5">
        {(Object.keys(ACTION_LABELS) as AdminDirectAction[]).map((a) => (
          <Button
            data-testid={`direct-action-${a}`}
            disabled={locked}
            key={a}
            onClick={() => set({ action: a })}
            size="sm"
            type="button"
            variant={a === draft.action ? "default" : "outline"}
          >
            {ACTION_LABELS[a]}
          </Button>
        ))}
      </div>
      {draft.action === "delete" ? (
        input("target", "Event id (hex), note1… or nevent1…", "font-mono")
      ) : (
        <AdminMemberPicker
          communityHost={communityHost}
          disabled={locked}
          member={draft.member}
          onChange={(member) => set({ member })}
        />
      )}
      {!frozen && memberState.status === "ok" && draft.action !== "delete" && (
        <MemberState member={memberState.data} />
      )}
      {!frozen && previewState.status === "ok" && draft.action === "delete" && (
        <EventPreview event={previewState.data} />
      )}
      {!frozen && lookupBlock && (
        <p
          className="text-xs text-destructive"
          data-testid="direct-lookup-error"
        >
          {lookupBlock}
        </p>
      )}
      {draft.action === "timeout" &&
        input("secs", "Duration (seconds)", "", "number")}
      {input("reason", "Reason (optional)")}
      <p
        className="text-xs text-muted-foreground"
        data-testid="direct-reason-audience"
      >
        {reasonAudienceCopy(frozen?.intent.action ?? draft.action)}
      </p>
      {error && (
        <p className="text-xs text-destructive" data-testid="direct-error">
          {error}
        </p>
      )}
      {frozen ? (
        <ConfirmStep />
      ) : (
        <Button
          data-testid="direct-review-btn"
          disabled={
            !c.canMutate ||
            c.submitting ||
            (lookup.status !== "ok" && lookup.status !== "idle") ||
            lookupBlock !== null
          }
          onClick={handleReview}
          size="sm"
          type="button"
        >
          Review
        </Button>
      )}
    </div>
  );
}

/** The frozen intent, leading with its community. */
function ConfirmStep({ readOnly = false }: { readOnly?: boolean }) {
  const c = useDirectActions();
  if (!c.frozen) return null;
  const { intent, community, name, member, preview } = c.frozen;
  return (
    <div
      className="space-y-2 rounded-md border border-border/60 px-3 py-2 text-xs"
      data-testid="direct-confirm"
    >
      <p className="flex flex-wrap items-center gap-1.5">
        In <CommunityBadge {...community} />
        <NotConnectedWarning host={intent.communityHost} />
      </p>
      <p>
        {ACTION_LABELS[intent.action]}{" "}
        {intent.action === "delete" ? (
          <code>{intent.target}</code>
        ) : (
          <span data-testid="direct-confirm-member">
            {name ? `${name} ` : ""}
            <code>
              {name
                ? `(${truncateNpub(intent.target)})`
                : truncateNpub(intent.target)}
            </code>
          </span>
        )}
        {intent.expirationSecs ? ` for ${intent.expirationSecs}s` : ""}?
      </p>
      {intent.action !== "delete" && (
        <PubKey
          pubkey={intent.target}
          testId="direct-confirm-npub"
          variant="full"
        />
      )}
      {member && <MemberState member={member} />}
      {preview && <EventPreview event={preview} />}
      <p data-testid="direct-confirm-reason">
        Reason: {intent.reason ?? "(none)"}
      </p>
      {c.pending && (
        <p className="text-muted-foreground" data-testid="direct-pending">
          Accepted; the relay is still applying it. Retry to check.
        </p>
      )}
      <div className="flex gap-1.5">
        {!readOnly && (
          <Button
            data-testid="direct-confirm-btn"
            disabled={!c.canMutate || c.submitting}
            onClick={() => void c.confirm()}
            size="sm"
            type="button"
            variant="destructive"
          >
            {c.error || c.pending ? "Retry" : "Confirm"}
          </Button>
        )}
        <Button
          data-testid="direct-discard-btn"
          disabled={c.submitting}
          onClick={c.discard}
          size="sm"
          type="button"
          variant="ghost"
        >
          Discard
        </Button>
      </div>
    </div>
  );
}

// ── Member picking ────────────────────────────────────────────────────────

/**
 * The admin search source: `GET /members/search` in `communityHost`, fenced
 * like the lookups. A pasted secret key never leaves the device.
 */
function useAdminMemberSearch(communityHost: string, query: string) {
  const c = useDirectActions();
  const secret = containsSecretKey(query);
  const state = useFencedLoad(
    query && !secret
      ? `${c.origin}\n${c.pubkey}\n${communityHost}\n${c.generation}\nsearch\n${query}`
      : null,
    async () =>
      (await searchAdminMembers(c.origin, communityHost, query)).items.map(
        (m): UserSearchResult => ({
          pubkey: m.pubkey,
          displayName: m.displayName,
          avatarUrl: m.avatarUrl,
          nip05Handle: m.nip05,
          ownerPubkey: null,
          isAgent: false,
        }),
      ),
  );
  return {
    secret,
    data: state.status === "ok" ? state.data : undefined,
    isLoading: state.status === "loading",
    error:
      state.status === "error"
        ? readErrorMessage(state.error, state.message)
        : null,
  };
}

/**
 * Search this community's profiles (including former members), or paste an
 * npub or hex key.
 */
export function AdminMemberPicker({
  communityHost,
  disabled,
  member,
  onChange,
}: {
  communityHost: string;
  disabled: boolean;
  member: UserSearchResult | null;
  onChange: (member: UserSearchResult | null) => void;
}) {
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query.trim());
  const search = useAdminMemberSearch(communityHost, deferred);
  const secret = search.secret || containsSecretKey(query);
  const candidates = useMemberCandidates(deferred, secret ? {} : search);

  if (member) {
    return (
      <div
        className="flex items-center gap-2"
        data-testid="direct-member-selected"
      >
        <span className="text-xs">{memberSearchLabel(member)}</span>
        <PubKey pubkey={member.pubkey} testId="direct-member-npub" />
        <Button
          className="h-auto p-0 text-xs"
          data-testid="direct-member-remove"
          disabled={disabled}
          onClick={() => onChange(null)}
          size="sm"
          type="button"
          variant="link"
        >
          Change
        </Button>
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
        placeholder="Search by name, or paste an npub or hex key"
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
      {search.error && !secret && (
        <p
          className="text-xs text-destructive"
          data-testid="direct-member-search-error"
        >
          {search.error}
        </p>
      )}
      {candidates.length > 0 && (
        <div className="rounded-md border border-border/60" role="listbox">
          {candidates.map((user) => (
            <MemberSearchResult
              key={user.pubkey}
              onSelect={() => {
                onChange(user);
                setQuery("");
              }}
              showKey
              testId={`direct-member-result-${user.pubkey}`}
              user={user}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/** Members section: pick someone, then ban or time them out. */
export function MembersSection({
  communityHost,
  onAct,
}: {
  communityHost: string;
  /** Called after prefilling the Actions draft, to show that section. */
  onAct: () => void;
}) {
  const c = useDirectActions();
  const [member, setMember] = useState<UserSearchResult | null>(null);
  return (
    <div className="space-y-2" data-testid="community-members">
      <AdminMemberPicker
        communityHost={communityHost}
        disabled={false}
        member={member}
        onChange={setMember}
      />
      {member && (
        <span className="flex gap-1">
          {(["ban", "timeout"] as const).map((action) => (
            <Button
              data-testid={`member-${action}`}
              disabled={!c.canMutate || c.frozen !== null}
              key={action}
              onClick={() => {
                c.setDraft({ ...emptyDraft(communityHost), action, member });
                onAct();
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              {action === "ban" ? "Ban" : "Time out"}
            </Button>
          ))}
        </span>
      )}
    </div>
  );
}
