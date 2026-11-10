/**
 * Main admin console panel — renders when probe state is `nip98Authorized` or
 * `disabled`.
 *
 * Shows four tabs: Reports (deployment-wide moderation reports), Feedback
 * (product feedback with optional image attachments), Communities (the
 * directory, and a page per community with its reports, restrictions, members
 * and direct actions), and Operators (Operator-only operator management).
 * With admin auth disabled the whole console, community pages included, is
 * read-only: every action control is disabled because the relay refuses writes.
 *
 * All query/UI state is keyed by `(pubkey, origin)`. In-flight native requests
 * are fenced by an effect-local `active` flag that is set to `false` in the
 * effect cleanup, ensuring stale results are discarded on arrival.
 *
 * Tauri invoke is not cancellable at the native layer, but the active-flag
 * pattern ensures stale results never update visible state or create
 * unreachable blob URLs.
 *
 * Sub-components live in adjacent files:
 *   - AdminConsolePanelHelpers.tsx  — AsyncState, useAsyncLoad, formatTimestamp,
 *                                     DetailRow, LoadingSpinner, ErrorMessage,
 *                                     AttachmentMeta, parseImetaAttachments
 *   - AdminConsoleFeedbackTab.tsx   — FeedbackTab, FeedbackDetail
 *   - AdminConsoleStaffingTab.tsx   — StaffingTab
 */

import { useEffect, useRef, useState } from "react";
import { Building2, MessageSquare, ShieldAlert, Users } from "lucide-react";
import { useCommunities } from "@/features/communities/useCommunities";
import { cn } from "@/shared/lib/cn";
import type { AdminPrincipalRole } from "./api";
import { ReportsTab } from "./AdminConsoleReportsTab";
import { FeedbackTab } from "./AdminConsoleFeedbackTab";
import { StaffingTab } from "./AdminConsoleStaffingTab";
import { DirectActionsProvider } from "./AdminConsoleActionsTab";
import { CommunitiesTab, CommunityPage } from "./AdminConsoleCommunities";
import {
  CommunityNavContext,
  type CommunityRef,
  useConnectedHost,
} from "./AdminConsoleCommunityBadge";

export {
  parseImetaAttachments,
  type AttachmentMeta,
} from "./AdminConsolePanelHelpers";

// ── Tab bar ───────────────────────────────────────────────────────────────

type Tab = "reports" | "feedback" | "communities" | "operators";

function TabBar({
  activeTab,
  onSelect,
  visible,
}: {
  activeTab: Tab;
  onSelect: (tab: Tab) => void;
  visible: Set<Tab>;
}) {
  const allTabs: Array<{
    value: Tab;
    label: string;
    Icon: React.ComponentType<{ className?: string }>;
  }> = [
    { value: "reports", label: "Reports", Icon: ShieldAlert },
    { value: "feedback", label: "Feedback", Icon: MessageSquare },
    { value: "communities", label: "Communities", Icon: Building2 },
    { value: "operators", label: "Operators", Icon: Users },
  ].filter((t) => visible.has(t.value as Tab)) as typeof allTabs;
  return (
    <div className="mb-4 flex gap-1 border-b border-border/60">
      {allTabs.map(({ value, label, Icon }) => (
        <button
          className={cn(
            "flex items-center gap-1.5 border-b-2 px-3 pb-2 pt-1 text-sm font-medium transition-colors",
            activeTab === value
              ? "border-primary text-foreground"
              : "border-transparent text-muted-foreground hover:text-foreground",
          )}
          data-testid={`admin-tab-${value}`}
          key={value}
          onClick={() => onSelect(value)}
          type="button"
        >
          <Icon className="h-3.5 w-3.5" />
          {label}
        </button>
      ))}
    </div>
  );
}

// ── Panel root ────────────────────────────────────────────────────────────

export function AdminConsolePanel({
  canMutate,
  origin,
  pubkey,
  role,
  initialTab,
  initialCommunity,
  onSelfMutation,
}: {
  /**
   * Whether mutation controls should be enabled. `false` when the relay probe
   * returned `disabled` — the admin API is accessible without credentials, so
   * the operator can read but must not be offered write affordances that could
   * accidentally mutate the relay without authentication.
   */
  canMutate: boolean;
  origin: string;
  /** Active identity pubkey — all state is keyed on (pubkey, origin). */
  pubkey: string;
  /** Principal role from probe — `"operator"` | `"moderator"` | undefined */
  role?: AdminPrincipalRole | null;
  /**
   * Called after a successful mutation that modified the current principal's
   * own operator row (self-demotion or self-removal). The parent should
   * re-probe the admin origin so the displayed role and visible tabs reflect
   * the new server state.
   */
  onSelfMutation?: () => void;
  /**
   * Override the initially active tab. Intended for unit tests that need to
   * land on a specific tab without driving click events through MinimalDocument.
   * Do not pass this prop in production code.
   */
  initialTab?: Tab;
  /** Test-only, like `initialTab`: land on this community's page. */
  initialCommunity?: CommunityRef;
}) {
  const isOperator = role === "operator";
  const [activeTab, setActiveTab] = useState<Tab>(initialTab ?? "reports");
  // Increment whenever the (pubkey, origin) context changes to invalidate all
  // in-flight useAsyncLoad effects via their effect-local `active` flags.
  const generationRef = useRef(0);
  const [generation, setGeneration] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey and origin are reactive props — effect fires when either changes to bump the generation fence
  useEffect(() => {
    generationRef.current += 1;
    setGeneration(generationRef.current);
  }, [pubkey, origin]);

  const [community, setCommunity] = useState<CommunityRef | null>(
    initialCommunity ?? null,
  );
  const { activeCommunity } = useCommunities();
  const connectedHost = useConnectedHost(activeCommunity?.relayUrl ?? "");
  const nav = {
    open: (next: CommunityRef) => {
      setCommunity(next);
      setActiveTab("communities");
    },
    connectedHost,
  };
  const visibleTabs = new Set<Tab>([
    "reports",
    "feedback",
    "communities",
    ...(isOperator ? (["operators"] as Tab[]) : []),
  ]);

  // Reset activeTab to the default when the current tab is no longer visible
  // (e.g. operator→moderator while Operators is selected).
  useEffect(() => {
    if (!visibleTabs.has(activeTab)) {
      setActiveTab("reports");
    }
  });

  return (
    <CommunityNavContext.Provider value={nav}>
      {/* The direct-action controller stays mounted across tabs and community
        pages, so a frozen, pending or in-flight action (and its requestId)
        survives navigation; an identity or origin change remounts it. */}
      <DirectActionsProvider
        canMutate={canMutate}
        generation={generation}
        key={`${pubkey}\n${origin}`}
        origin={origin}
        pubkey={pubkey}
      >
        <div
          className="flex min-h-0 flex-1 flex-col"
          data-testid="admin-console-panel"
        >
          <TabBar
            activeTab={activeTab}
            onSelect={(tab) => {
              if (tab === "communities") setCommunity(null);
              setActiveTab(tab);
            }}
            visible={visibleTabs}
          />
          {activeTab === "reports" && (
            <div data-testid="reports-tab">
              <ReportsTab
                canMutate={canMutate}
                origin={origin}
                pubkey={pubkey}
                generation={generation}
              />
            </div>
          )}
          {activeTab === "feedback" && (
            <FeedbackTab
              canMutate={canMutate}
              origin={origin}
              pubkey={pubkey}
              generation={generation}
            />
          )}
          {activeTab === "communities" &&
            (community ? (
              <CommunityPage
                canMutate={canMutate}
                community={community}
                generation={generation}
                key={community.id}
                onBack={() => setCommunity(null)}
                origin={origin}
                pubkey={pubkey}
              />
            ) : (
              <CommunitiesTab generation={generation} origin={origin} />
            ))}
          {activeTab === "operators" && isOperator && (
            <StaffingTab
              origin={origin}
              pubkey={pubkey}
              generation={generation}
              onSelfMutation={onSelfMutation}
            />
          )}
        </div>
      </DirectActionsProvider>
    </CommunityNavContext.Provider>
  );
}
