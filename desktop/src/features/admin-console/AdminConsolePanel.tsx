/**
 * Main admin console panel — renders when probe state is `nip98Authorized` or
 * `disabled`.
 *
 * Shows three tabs: Reports (deployment-wide moderation reports), Feedback
 * (product feedback with optional image attachments), and Staffing (Operator-
 * only operator management).
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
import { MessageSquare, ShieldAlert, Users } from "lucide-react";
import { cn } from "@/shared/lib/cn";
import type { AdminPrincipalRole } from "./api";
import { ReportsTab } from "./AdminConsoleReportsTab";
import { FeedbackTab } from "./AdminConsoleFeedbackTab";
import { StaffingTab } from "./AdminConsoleStaffingTab";

export {
  parseImetaAttachments,
  type AttachmentMeta,
} from "./AdminConsolePanelHelpers";

// ── Tab bar ───────────────────────────────────────────────────────────────

type Tab = "reports" | "feedback" | "staffing";

function TabBar({
  activeTab,
  onSelect,
  showStaffing,
}: {
  activeTab: Tab;
  onSelect: (tab: Tab) => void;
  showStaffing: boolean;
}) {
  const allTabs: Array<{
    value: Tab;
    label: string;
    Icon: React.ComponentType<{ className?: string }>;
  }> = [
    { value: "reports", label: "Reports", Icon: ShieldAlert },
    { value: "feedback", label: "Feedback", Icon: MessageSquare },
    ...(showStaffing
      ? [{ value: "staffing" as const, label: "Staffing", Icon: Users }]
      : []),
  ];
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
   * Override the initially active tab. Intended for unit tests that need to
   * land on a specific tab without driving click events through MinimalDocument.
   * Do not pass this prop in production code.
   */
  initialTab?: Tab;
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

  // Reset activeTab to the default when the current tab is no longer visible
  // for the current role (e.g. operator→moderator while Staffing is selected).
  // Guard is written against tab-visibility (the set TabBar would render for
  // this role) rather than a hard-coded role string so it generalises to
  // future tabs without requiring a companion role check here.
  useEffect(() => {
    const visibleTabs = new Set<Tab>([
      "reports",
      "feedback",
      ...(isOperator ? (["staffing"] as Tab[]) : []),
    ]);
    if (!visibleTabs.has(activeTab)) {
      setActiveTab("reports");
    }
  }, [isOperator, activeTab]);

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="admin-console-panel"
    >
      <TabBar
        activeTab={activeTab}
        onSelect={setActiveTab}
        showStaffing={isOperator}
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
      {activeTab === "staffing" && isOperator && (
        <StaffingTab
          canMutate={canMutate}
          origin={origin}
          pubkey={pubkey}
          generation={generation}
        />
      )}
    </div>
  );
}
