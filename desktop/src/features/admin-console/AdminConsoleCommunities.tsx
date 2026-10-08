/**
 * Communities tab and the per-community page.
 *
 * The tab searches the relay's community directory by host prefix and pins
 * the connected community, resolved natively from the active relay and
 * matched exactly, independent of whichever directory page is loaded. A
 * community page shows that community's Reports, Restrictions, Members and
 * Actions. Every section names the community by the page's host; nothing is
 * derived from the active relay.
 */

import { useDeferredValue, useMemo, useRef, useState } from "react";
import { ArrowLeft, LoaderCircle } from "lucide-react";
import { containsSecretKey } from "@/features/onboarding/lib/keyImportInput";
import { Button } from "@/shared/ui/button";
import { listAdminCommunities, type AdminCommunityDto } from "./api";
import { ActionsSection, MembersSection } from "./AdminConsoleActionsTab";
import {
  CommunityBadge,
  type CommunityRef,
  NotConnectedWarning,
  useCommunityNav,
} from "./AdminConsoleCommunityBadge";
import {
  adminErrorMessage,
  adminRouteUnsupported,
  ErrorMessage,
  LoadingSpinner,
  UNSUPPORTED_BROWSING,
  useAsyncLoad,
} from "./AdminConsolePanelHelpers";
import { ReportsTab } from "./AdminConsoleReportsTab";
import { RestrictionsSection } from "./AdminConsoleStaffingTab";

// ── Communities tab ───────────────────────────────────────────────────────

export function CommunitiesTab({
  origin,
  generation,
}: {
  origin: string;
  generation: number;
}) {
  const { open, connectedHost } = useCommunityNav();
  const [query, setQuery] = useState("");
  const q = useDeferredValue(query.trim().toLowerCase());
  // A pasted secret key never leaves the device.
  const secret = containsSecretKey(q);
  // The raw input leads the deferred query, so pagination checks both: an
  // older page's cursor must never carry newly typed key material.
  const blocked = secret || containsSecretKey(query);
  // One identity per search transition, so A→B→A is three searches and a
  // page requested under an earlier one can never land under a later one.
  const key = `${origin}\n${q}\n${generation}`;
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new identity per key change is the point
  const search = useMemo(() => ({}), [key]);
  const searchRef = useRef(search);
  searchRef.current = search;
  const [more, setMore] = useState<{
    search: object;
    items: AdminCommunityDto[];
    nextCursor: string | null;
  } | null>(null);
  const [moreState, setMoreState] = useState<{
    search: object;
    busy: boolean;
    error: string | null;
  } | null>(null);

  const first = useAsyncLoad(
    () =>
      secret
        ? Promise.resolve({ items: [], nextCursor: null })
        : listAdminCommunities(origin, q),
    [origin, q],
    generation,
  );
  // The connected community, by exact host match, whatever page is loaded.
  const pinned = useAsyncLoad(
    async () =>
      connectedHost
        ? ((await listAdminCommunities(origin, connectedHost)).items.find(
            (c) => c.host === connectedHost,
          ) ?? null)
        : null,
    [origin, connectedHost],
    generation,
  );

  if (first.status === "error" && adminRouteUnsupported(first.error)) {
    return (
      <p
        className="text-sm text-muted-foreground"
        data-testid="communities-unsupported"
      >
        {UNSUPPORTED_BROWSING}
      </p>
    );
  }

  const extra = more?.search === search ? more : null;
  const moreStatus = moreState?.search === search ? moreState : null;
  // Pinned only while it matches the search the way the relay does: a
  // case-insensitive host prefix.
  const pinnedRow =
    pinned.status === "ok" && pinned.data?.host.toLowerCase().startsWith(q)
      ? pinned.data
      : null;
  const items =
    first.status === "ok"
      ? [...first.data.items, ...(extra?.items ?? [])].filter(
          (c) => c.id !== pinnedRow?.id,
        )
      : [];
  const nextCursor = extra
    ? extra.nextCursor
    : first.status === "ok"
      ? first.data.nextCursor
      : null;

  const loadMore = async () => {
    if (!nextCursor || blocked) return;
    setMoreState({ search, busy: true, error: null });
    try {
      const page = await listAdminCommunities(origin, q, nextCursor);
      if (searchRef.current !== search) return;
      setMore((prev) => ({
        search,
        items: [...(prev?.search === search ? prev.items : []), ...page.items],
        nextCursor: page.nextCursor,
      }));
      setMoreState({ search, busy: false, error: null });
    } catch (e) {
      if (searchRef.current !== search) return;
      setMoreState({ search, busy: false, error: adminErrorMessage(e) });
    }
  };

  const row = (c: AdminCommunityDto, connected: boolean) => (
    <li key={c.id}>
      <button
        className="flex w-full items-center gap-2 rounded-md border border-border/60 px-3 py-2 text-left text-sm hover:bg-muted/40"
        data-testid={`community-row-${c.host}`}
        onClick={() => open(c)}
        type="button"
      >
        <span className="flex-1 truncate">{c.host}</span>
        {connected && (
          <span className="text-xs text-muted-foreground">Connected</span>
        )}
      </button>
    </li>
  );

  return (
    <div className="space-y-3" data-testid="communities-tab">
      <input
        className="w-full rounded-md border border-border/60 bg-background px-2 py-1 text-xs"
        data-testid="communities-search-input"
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search by host (e.g. team.example.com)"
        value={query}
      />
      {pinnedRow && (
        <ul data-testid="communities-pinned">{row(pinnedRow, true)}</ul>
      )}
      {secret && (
        <p
          className="text-xs text-destructive"
          data-testid="communities-search-secret"
        >
          That's a secret key. Never paste it here.
        </p>
      )}
      {first.status === "loading" && <LoadingSpinner />}
      {first.status === "error" && <ErrorMessage message={first.message} />}
      {first.status === "ok" && items.length === 0 && !pinnedRow && !secret && (
        <p className="text-sm text-muted-foreground">No communities found.</p>
      )}
      <ul className="space-y-1">{items.map((c) => row(c, false))}</ul>
      {moreStatus?.error && <ErrorMessage message={moreStatus.error} />}
      {nextCursor && !blocked && (
        <Button
          data-testid="communities-load-more"
          disabled={moreStatus?.busy ?? false}
          onClick={() => void loadMore()}
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

// ── Community page ────────────────────────────────────────────────────────

const SECTIONS = ["reports", "restrictions", "members", "actions"] as const;
type Section = (typeof SECTIONS)[number];

export function CommunityPage({
  community,
  canMutate,
  origin,
  pubkey,
  generation,
  onBack,
}: {
  community: CommunityRef;
  canMutate: boolean;
  origin: string;
  pubkey: string;
  generation: number;
  onBack: () => void;
}) {
  const [section, setSection] = useState<Section>("reports");
  return (
    <div className="space-y-3" data-testid="community-page">
      <div
        className="flex flex-wrap items-center gap-2 rounded-md border border-border/60 px-3 py-2"
        data-testid="community-banner"
      >
        <Button
          className="h-7 px-2"
          data-testid="community-back"
          onClick={onBack}
          size="sm"
          type="button"
          variant="ghost"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
        </Button>
        <CommunityBadge
          host={community.host}
          icon={community.icon}
          id={community.id}
        />
        <NotConnectedWarning host={community.host} />
      </div>
      <div className="flex gap-1">
        {SECTIONS.map((s) => (
          <Button
            data-testid={`community-section-${s}`}
            key={s}
            onClick={() => setSection(s)}
            size="sm"
            type="button"
            variant={s === section ? "default" : "outline"}
          >
            {s[0].toUpperCase() + s.slice(1)}
          </Button>
        ))}
      </div>
      {section === "reports" && (
        <ReportsTab
          canMutate={canMutate}
          communityId={community.id}
          generation={generation}
          origin={origin}
          pubkey={pubkey}
        />
      )}
      {section === "restrictions" && (
        <RestrictionsSection
          canMutate={canMutate}
          communityHost={community.host}
          generation={generation}
          origin={origin}
          pubkey={pubkey}
        />
      )}
      {section === "members" && (
        <MembersSection
          communityHost={community.host}
          onAct={() => setSection("actions")}
        />
      )}
      {section === "actions" && <ActionsSection community={community} />}
    </div>
  );
}
