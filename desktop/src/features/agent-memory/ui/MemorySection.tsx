import * as React from "react";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  RefreshCw,
} from "lucide-react";

import {
  useAgentMemoryGraph,
  useIsManagedAgent,
} from "@/features/agent-memory/hooks";
import type {
  DanglingRef,
  MemoryTreeNode,
} from "@/features/agent-memory/lib/buildMemoryGraph";
import type { EngramEntry } from "@/shared/api/tauriEngrams";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

/**
 * Memory section — IXI-7 phase 1 read-only viewer.
 *
 * Owner-gated: returns `null` for non-owners (and while the managed-agent
 * list is still loading, to avoid a one-frame flash of the section before
 * we know the viewer's owner status).
 *
 * The whole thing is contained: skeleton/error/empty live *inside* this
 * section so the rest of the profile panel stays interactive while we
 * fetch+decrypt. Refetch is non-blocking (cached data stays visible while
 * `isFetching` is true).
 *
 * Layout:
 *   [ Memory                                     ↻ refetch ]
 *   ⚠ truncated relay banner (if applicable)
 *   ── tree rooted at `core` ──
 *   ── orphans list (if any) ──
 *   ── dangling refs list (if any) ──
 *
 * tho will refine the visual design — this is the structural placement.
 */
export function MemorySection({
  agentPubkey,
}: {
  agentPubkey: string;
}): React.ReactElement | null {
  const isOwner = useIsManagedAgent(agentPubkey);

  // Hide entirely for non-owners. Defer (`null`) while the managed-agent
  // list is still loading rather than render a placeholder we'll yank.
  if (isOwner !== true) return null;

  return <MemorySectionForOwner agentPubkey={agentPubkey} />;
}

function MemorySectionForOwner({ agentPubkey }: { agentPubkey: string }) {
  const { query, graph } = useAgentMemoryGraph(agentPubkey);

  // Order matters here. We want:
  // - first paint, no cache → skeleton
  // - error with no cache → error state (with retry)
  // - error WITH cache → keep the data, show a non-blocking "refetch failed"
  //   banner; user can retry without losing what they had
  // - data, but empty → empty state ("This agent has no memories yet")
  // - data, non-empty → render
  const showInitialSkeleton = query.isLoading && !query.data;
  const showInitialError = query.isError && !query.data;

  return (
    <section className="mt-4" data-testid="agent-memory-section">
      <div className="mb-2 flex items-center justify-between">
        <h4 className="text-xs font-medium uppercase tracking-wider text-muted-foreground/70">
          Memory
        </h4>
        {query.data ? (
          <button
            aria-label="Refresh memory"
            className={cn(
              "rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground",
              query.isFetching && "cursor-wait",
            )}
            data-testid="agent-memory-refetch"
            disabled={query.isFetching}
            onClick={() => query.refetch()}
            type="button"
          >
            <RefreshCw
              className={cn("h-3.5 w-3.5", query.isFetching && "animate-spin")}
            />
          </button>
        ) : null}
      </div>

      {showInitialSkeleton ? <MemorySkeleton /> : null}

      {showInitialError ? (
        <MemoryErrorState
          error={query.error}
          onRetry={() => query.refetch()}
          retrying={query.isFetching}
        />
      ) : null}

      {query.data && graph ? (
        <>
          {/* Stale-cache error banner: shown when a refetch fails but we
              still have prior data on screen. Distinct from the initial
              error state above. */}
          {query.isError && !query.isFetching ? (
            <MemoryStaleErrorBanner onRetry={() => query.refetch()} />
          ) : null}

          {query.data.truncated ? <MemoryTruncatedBanner /> : null}

          <MemoryGraphView graph={graph} />
        </>
      ) : null}
    </section>
  );
}

// ── Subviews ────────────────────────────────────────────────────────────────

function MemorySkeleton() {
  return (
    <div
      aria-label="Loading memory"
      className="space-y-2"
      data-testid="agent-memory-skeleton"
      role="status"
    >
      <Skeleton className="h-4 w-2/3" />
      <Skeleton className="h-4 w-1/2" />
      <Skeleton className="h-4 w-3/5" />
    </div>
  );
}

function MemoryErrorState({
  error,
  onRetry,
  retrying,
}: {
  error: unknown;
  onRetry: () => void;
  retrying: boolean;
}) {
  const message =
    error instanceof Error ? error.message : String(error ?? "unknown error");
  return (
    <div
      className="flex flex-col gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-xs"
      data-testid="agent-memory-error"
      role="alert"
    >
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-destructive" />
        <div className="space-y-1">
          <div className="font-medium text-destructive">
            Couldn't load memory
          </div>
          <div className="text-muted-foreground">{message}</div>
        </div>
      </div>
      <Button
        className="self-start"
        disabled={retrying}
        onClick={onRetry}
        size="sm"
        variant="outline"
      >
        {retrying ? "Retrying…" : "Retry"}
      </Button>
    </div>
  );
}

function MemoryStaleErrorBanner({ onRetry }: { onRetry: () => void }) {
  return (
    <div
      className="mb-2 flex items-center gap-2 rounded-md border border-warning/30 bg-warning/5 px-2 py-1.5 text-xs"
      data-testid="agent-memory-stale-error"
    >
      <AlertTriangle className="h-3 w-3 shrink-0 text-warning" />
      <span className="flex-1 text-muted-foreground">Refresh failed.</span>
      <button
        className="font-medium text-warning hover:underline"
        onClick={onRetry}
        type="button"
      >
        Retry
      </button>
    </div>
  );
}

function MemoryTruncatedBanner() {
  return (
    <div
      className="mb-2 flex items-start gap-2 rounded-md border border-warning/30 bg-warning/5 p-2 text-xs"
      data-testid="agent-memory-truncated"
    >
      <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0 text-warning" />
      <span className="text-muted-foreground">
        This list may be incomplete — the relay returned the maximum number of
        memories.
      </span>
    </div>
  );
}

function MemoryGraphView({
  graph,
}: {
  graph: NonNullable<ReturnType<typeof useAgentMemoryGraph>["graph"]>;
}) {
  const { rootedTree, orphans, dangling } = graph;

  const isEmpty = !rootedTree && orphans.length === 0 && dangling.length === 0;
  if (isEmpty) {
    return (
      <p
        className="text-sm italic text-muted-foreground"
        data-testid="agent-memory-empty"
      >
        This agent has no memories yet.
      </p>
    );
  }

  return (
    <div className="space-y-3">
      {rootedTree ? (
        <div data-testid="agent-memory-tree">
          <TreeNode node={rootedTree} depth={0} />
        </div>
      ) : // No `core` but there are memories. Surface a small note so the
      // user understands why the orphans-only view looks the way it does.
      // This branch only fires when `orphans.length > 0` by design: the
      // `isEmpty` short-circuit above (line ~220) catches the
      // zero-memories-and-zero-core case with the empty-state copy, so we
      // don't need a redundant `orphans.length === 0` branch here.
      orphans.length > 0 ? (
        <p
          className="text-xs italic text-muted-foreground"
          data-testid="agent-memory-no-core"
        >
          No <code className="font-mono text-[10px]">core</code> memory yet —
          agent identity is unrooted.
        </p>
      ) : null}

      {orphans.length > 0 ? (
        <div data-testid="agent-memory-orphans">
          <h5 className="mb-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/60">
            Unreferenced ({orphans.length})
          </h5>
          <div className="space-y-1">
            {orphans.map((entry) => (
              <EntryDisclosure depth={0} entry={entry} key={entry.eventId} />
            ))}
          </div>
        </div>
      ) : null}

      {dangling.length > 0 ? (
        <div data-testid="agent-memory-dangling">
          <h5 className="mb-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground/60">
            Broken references ({dangling.length})
          </h5>
          <ul className="space-y-1 text-xs">
            {dangling.map((d) => (
              <DanglingRefRow danglingRef={d} key={d.slug} />
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

function TreeNode({ node, depth }: { node: MemoryTreeNode; depth: number }) {
  return (
    <div className="space-y-1">
      <EntryDisclosure depth={depth} entry={node.entry} />
      {node.children.length > 0 ? (
        <div className="space-y-1">
          {node.children.map((child) => (
            <TreeNode
              depth={depth + 1}
              key={child.entry.eventId}
              node={child}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** A single engram, click to expand body. */
function EntryDisclosure({
  depth,
  entry,
}: {
  depth: number;
  entry: EngramEntry;
}) {
  // Auto-expand the root (`core`) so the user always sees something on first
  // open. Everything else is collapsed by default to keep the panel tidy.
  const [open, setOpen] = React.useState(depth === 0 && entry.slug === "core");
  const indent = Math.min(depth, 4); // cap visual nesting

  return (
    <div className="text-xs" style={{ paddingLeft: `${indent * 12}px` }}>
      <button
        className="flex w-full items-start gap-1 rounded-sm px-1 py-0.5 text-left hover:bg-muted/40"
        onClick={() => setOpen((v) => !v)}
        type="button"
      >
        {open ? (
          <ChevronDown className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
        )}
        <span className="min-w-0 flex-1 truncate">
          <span className="font-mono text-[11px] text-foreground">
            {entry.slug}
          </span>
          {!open ? (
            <span className="ml-2 text-muted-foreground/70">
              {bodyPreview(entry.body)}
            </span>
          ) : null}
        </span>
      </button>
      {open ? (
        <div className="mt-1 ml-4 whitespace-pre-wrap break-words rounded-md bg-muted/30 p-2 text-[11px] leading-relaxed text-muted-foreground">
          {entry.body || (
            <span className="italic text-muted-foreground/60">(empty)</span>
          )}
        </div>
      ) : null}
    </div>
  );
}

function DanglingRefRow({ danglingRef }: { danglingRef: DanglingRef }) {
  return (
    <li className="flex items-baseline gap-2">
      <span className="font-mono text-[11px] text-warning">
        {danglingRef.slug}
      </span>
      <span className="text-muted-foreground/70">
        ← {danglingRef.referencedBy.join(", ")}
      </span>
    </li>
  );
}

function bodyPreview(body: string): string {
  const trimmed = body.trim().replace(/\s+/g, " ");
  if (trimmed.length === 0) return "(empty)";
  if (trimmed.length <= 60) return trimmed;
  return `${trimmed.slice(0, 60)}…`;
}
