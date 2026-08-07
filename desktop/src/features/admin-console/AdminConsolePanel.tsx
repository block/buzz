/**
 * Main admin console panel — renders when probe state is `nip98Authorized` or
 * `disabled`.
 *
 * Shows two tabs: Reports (deployment-wide moderation reports) and Feedback
 * (product feedback with optional image attachments).
 *
 * All query/UI state is keyed by `(pubkey, origin)`. In-flight native requests
 * are fenced by an effect-local `active` flag that is set to `false` in the
 * effect cleanup, ensuring stale results are discarded on arrival.
 *
 * Tauri invoke is not cancellable at the native layer, but the active-flag
 * pattern ensures stale results never update visible state or create
 * unreachable blob URLs.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import {
  AlertCircle,
  ChevronLeft,
  Download,
  LoaderCircle,
  MessageSquare,
  ShieldAlert,
} from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  fetchAdminAttachmentBlobUrl,
  getAdminFeedback,
  getAdminReport,
  listAdminFeedback,
  listAdminReports,
  type AdminAttachmentErrorCode,
  type AdminFeedbackDto,
  type AdminFeedbackSummaryDto,
  type AdminReportDetailDto,
  type AdminReportDto,
} from "./api";
import { formatRelativeTime } from "../forum/lib/time";

// ── Generic async state ───────────────────────────────────────────────────

type AsyncState<T> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ok"; data: T }
  | { status: "error"; message: string };

/**
 * Async load hook with effect-local active-flag cancellation.
 *
 * Each effect invocation sets `active = true` and flips it to `false` in the
 * cleanup function. Completions check `active` before calling setState, so a
 * result that arrives after the deps changed (or the component unmounted) is
 * silently discarded.
 *
 * `load` is stored in a ref so it is not a dependency of the effect — callers
 * create it inline and `deps` + `generation` are the explicit trigger list.
 */
function useAsyncLoad<T>(
  load: () => Promise<T>,
  deps: unknown[],
  generation: number,
): AsyncState<T> {
  const [state, setState] = useState<AsyncState<T>>({ status: "idle" });
  const loadRef = useRef(load);
  loadRef.current = load;

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadRef is a stable ref; deps and generation are the intentional trigger set
  useEffect(() => {
    let active = true;
    setState({ status: "loading" });
    loadRef.current().then(
      (data) => {
        if (!active) return;
        setState({ status: "ok", data });
      },
      (e: unknown) => {
        if (!active) return;
        setState({
          status: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      },
    );
    return () => {
      active = false;
    };
  }, [...deps, generation]);

  return state;
}

// ── Structured detail helpers ────────────────────────────────────────────

/** One label/value row in a detail view. Null/empty values render as "—". */
function DetailRow({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string | null | undefined;
  mono?: boolean;
}) {
  const display = value != null && value !== "" ? value : "—";
  return (
    <div className="grid grid-cols-[9rem_1fr] gap-x-3 gap-y-0.5 text-xs">
      <span className="font-medium text-muted-foreground">{label}</span>
      <span className={cn("break-all", mono && "font-mono")}>{display}</span>
    </div>
  );
}

function formatTimestamp(raw: string | null | undefined): string {
  if (raw == null || raw === "") return "—";
  // DTO timestamps are ISO-8601 strings (DateTime<Utc> via serde).
  // Parse to a Date, convert to unix seconds, then format relatively.
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) return raw;
  const absolute = date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  const rel = formatRelativeTime(Math.floor(date.getTime() / 1000));
  // Render relative label with the absolute value inline in parentheses.
  return `${rel} (${absolute})`;
}

function statusVariant(
  status: string,
): "success" | "warning" | "secondary" | "outline" {
  switch (status.toLowerCase()) {
    case "open":
      return "warning";
    case "resolved":
    case "dismissed":
      return "success";
    default:
      return "secondary";
  }
}

// ── imeta attachment parsing ──────────────────────────────────────────────

/**
 * Validated attachment metadata parsed from a feedback detail's `tags` field.
 * The relay serialises `AdminFeedback` with `serde(rename_all = "camelCase")`,
 * so the wire shape is `{ ..., tags: string[][] }`.
 */
type AttachmentMeta = {
  /** Lowercase 64-hex SHA-256 as stored/returned by the relay. */
  sha256: string;
  /** MIME type from the `m` imeta field. */
  mime: string;
  /** Byte size from the `size` imeta field. */
  size: number;
};

/**
 * Parse imeta attachment metadata from the relay's `tags: string[][]` wire
 * format. Matches the reference SPA implementation in `admin-web/src/App.tsx`.
 *
 * Each `imeta` tag looks like:
 *   `["imeta", "url https://...", "m image/png", "x <sha256>", "size 12345"]`
 * Each entry after `"imeta"` is a singleton `"key value"` string.
 *
 * Rejected: missing x/m/size, non-lowercase-hex x, non-positive size.
 */
export function parseImetaAttachments(tags: unknown): AttachmentMeta[] {
  if (!Array.isArray(tags)) return [];
  const result: AttachmentMeta[] = [];
  for (const tag of tags) {
    if (!Array.isArray(tag) || tag[0] !== "imeta") continue;
    const values = new Map<string, string>();
    for (const entry of (tag as string[]).slice(1)) {
      const sep = typeof entry === "string" ? entry.indexOf(" ") : -1;
      if (sep > 0) {
        values.set(entry.slice(0, sep), entry.slice(sep + 1));
      }
    }
    const sha256 = values.get("x") ?? "";
    const mime = values.get("m") ?? "";
    const rawSize = values.get("size") ?? "";
    const size = Number(rawSize);
    // Require exactly 64 lowercase hex chars for the hash (relay stores lowercase;
    // uppercase returns 404). Require a non-empty MIME type and a positive size.
    if (
      sha256.length !== 64 ||
      !/^[0-9a-f]{64}$/.test(sha256) ||
      !mime ||
      !Number.isFinite(size) ||
      size <= 0
    ) {
      continue;
    }
    result.push({ sha256, mime, size });
  }
  return result;
}

// ── Reports tab ───────────────────────────────────────────────────────────

function ReportsTab({
  origin,
  pubkey,
  generation,
}: {
  origin: string;
  pubkey: string;
  generation: number;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const listState = useAsyncLoad(
    () => listAdminReports(origin),
    [origin, pubkey],
    generation,
  );

  if (selectedId) {
    return (
      <ReportDetail
        origin={origin}
        pubkey={pubkey}
        generation={generation}
        reportId={selectedId}
        onBack={() => setSelectedId(null)}
      />
    );
  }

  if (listState.status === "loading") {
    return <LoadingSpinner />;
  }
  if (listState.status === "error") {
    return <ErrorMessage message={listState.message} />;
  }
  if (listState.status !== "ok") return null;

  const reports = listState.data;
  if (!Array.isArray(reports) || reports.length === 0) {
    return <p className="text-sm text-muted-foreground">No reports found.</p>;
  }

  return (
    <ul className="space-y-1">
      {reports.map((report: AdminReportDto) => {
        const id = report.id;
        const summary = report.reportType || "Report";
        const status = report.status;
        return (
          <li key={id}>
            <button
              className="w-full rounded-md border border-border/60 px-3 py-2.5 text-left text-sm hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => setSelectedId(id)}
              type="button"
            >
              <span className="block font-medium">{summary}</span>
              {status && (
                <span className="text-xs text-muted-foreground">{status}</span>
              )}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function ReportFields({ data }: { data: AdminReportDetailDto }) {
  const status = data.status ?? "";
  return (
    <div
      className="space-y-1.5 rounded-md border border-border/60 px-3 py-2.5"
      data-testid="report-detail-fields"
    >
      <div className="mb-2 flex flex-wrap items-center gap-2">
        {status && <Badge variant={statusVariant(status)}>{status}</Badge>}
        {data.reportType && <Badge variant="outline">{data.reportType}</Badge>}
      </div>
      <DetailRow label="ID" value={data.id} mono />
      <DetailRow label="Community" value={data.communityId} mono />
      <DetailRow label="Host" value={data.communityHost} />
      <DetailRow label="Event ID" value={data.reportEventId} mono />
      <DetailRow label="Reporter" value={data.reporterPubkey} mono />
      <DetailRow label="Target kind" value={data.targetKind} />
      <DetailRow label="Target" value={data.target} mono />
      <DetailRow label="Channel" value={data.channelId ?? null} mono />
      <DetailRow label="Note" value={data.note ?? null} />
      <DetailRow label="Resolved by" value={data.resolvedBy ?? null} mono />
      <DetailRow label="Resolved at" value={formatTimestamp(data.resolvedAt)} />
      <DetailRow label="Action ID" value={data.actionId ?? null} mono />
      <DetailRow label="Created" value={formatTimestamp(data.createdAt)} />
      {data.message != null && (
        <div className="mt-3 space-y-1.5 rounded-md border border-border/40 px-3 py-2.5">
          <p className="mb-1 text-xs font-semibold text-muted-foreground">
            Reported message
            {data.message.deletedAt != null && (
              <span className="ml-1.5 text-destructive">(deleted)</span>
            )}
          </p>
          <DetailRow label="Author" value={data.message.authorPubkey} mono />
          <DetailRow label="Content" value={data.message.content} />
          <DetailRow
            label="Msg created"
            value={formatTimestamp(data.message.createdAt)}
          />
        </div>
      )}
    </div>
  );
}

function ReportDetail({
  origin,
  pubkey,
  generation,
  reportId,
  onBack,
}: {
  origin: string;
  pubkey: string;
  generation: number;
  reportId: string;
  onBack: () => void;
}) {
  const detailState = useAsyncLoad(
    () => getAdminReport(origin, reportId),
    [origin, pubkey, reportId],
    generation,
  );

  return (
    <div>
      <button
        className="mb-3 flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        onClick={onBack}
        type="button"
      >
        <ChevronLeft className="h-3.5 w-3.5" />
        Back to reports
      </button>
      {detailState.status === "loading" && <LoadingSpinner />}
      {detailState.status === "error" && (
        <ErrorMessage message={detailState.message} />
      )}
      {detailState.status === "ok" && <ReportFields data={detailState.data} />}
    </div>
  );
}

// ── Feedback tab ──────────────────────────────────────────────────────────

function FeedbackTab({
  origin,
  pubkey,
  generation,
}: {
  origin: string;
  pubkey: string;
  generation: number;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const listState = useAsyncLoad(
    () => listAdminFeedback(origin),
    [origin, pubkey],
    generation,
  );

  if (selectedId) {
    return (
      <FeedbackDetail
        feedbackId={selectedId}
        onBack={() => setSelectedId(null)}
        origin={origin}
        pubkey={pubkey}
        generation={generation}
      />
    );
  }

  if (listState.status === "loading") return <LoadingSpinner />;
  if (listState.status === "error") {
    return <ErrorMessage message={listState.message} />;
  }
  if (listState.status !== "ok") return null;

  const items = listState.data;
  if (!Array.isArray(items) || items.length === 0) {
    return <p className="text-sm text-muted-foreground">No feedback found.</p>;
  }

  return (
    <ul className="space-y-1">
      {items.map((item: AdminFeedbackSummaryDto) => {
        const id = item.id;
        const text = item.bodySummary.slice(0, 120);
        const receivedAt = item.receivedAt;
        return (
          <li key={id}>
            <button
              className="w-full rounded-md border border-border/60 px-3 py-2.5 text-left text-sm hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => setSelectedId(id)}
              type="button"
            >
              <span className="block font-medium line-clamp-2">{text}</span>
              {receivedAt && (
                <span className="text-xs text-muted-foreground">
                  {formatTimestamp(receivedAt)}
                </span>
              )}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

// ── Attachment viewer ─────────────────────────────────────────────────────

function AttachmentViewer({
  origin,
  pubkey,
  feedbackId,
  attachment,
  panelGeneration,
}: {
  origin: string;
  pubkey: string;
  feedbackId: string;
  attachment: AttachmentMeta;
  /** Generation from the parent panel — when this changes the attachment
   *  context has changed and any in-flight load result is stale. */
  panelGeneration: number;
}) {
  const [blobUrl, setBlobUrl] = useState<string | null>(null);
  const [error, setError] = useState<AdminAttachmentErrorCode | null>(null);
  const [loading, setLoading] = useState(false);
  const blobUrlRef = useRef<string | null>(null);
  // Per-load generation: incremented when a new load starts AND in cleanup so
  // that unmount or panelGeneration change invalidates any in-flight load.
  const loadGenRef = useRef(0);
  // Keep current origin/pubkey in refs so the callback can compare against
  // the rendered-at-call-time values without capturing stale closure copies.
  const originRef = useRef(origin);
  const pubkeyRef = useRef(pubkey);
  originRef.current = origin;
  pubkeyRef.current = pubkey;

  // On panelGeneration change (identity/origin switch) or unmount:
  // invalidate any in-flight load and revoke the cached blob URL.
  // biome-ignore lint/correctness/useExhaustiveDependencies: panelGeneration is a prop that drives cleanup re-registration; the cleanup body mutates refs, not reactive state
  useEffect(() => {
    return () => {
      // Increment generation so any in-flight native callback sees a mismatch.
      loadGenRef.current += 1;
      if (blobUrlRef.current) {
        URL.revokeObjectURL(blobUrlRef.current);
        blobUrlRef.current = null;
      }
    };
  }, [panelGeneration]);

  const load = useCallback(async () => {
    // Capture snapshot of context at the moment this load starts.
    const thisGen = ++loadGenRef.current;
    const thisOrigin = origin;
    const thisPubkey = pubkey;

    setLoading(true);
    setError(null);
    try {
      const url = await fetchAdminAttachmentBlobUrl(
        origin,
        feedbackId,
        attachment.sha256,
        attachment.mime,
        attachment.size,
      );

      // Discard if a newer load started, the component was unmounted/context
      // changed (loadGenRef incremented in cleanup), or origin/pubkey differ.
      if (
        thisGen !== loadGenRef.current ||
        thisOrigin !== originRef.current ||
        thisPubkey !== pubkeyRef.current
      ) {
        URL.revokeObjectURL(url);
        return;
      }

      // Revoke any previous blob before replacing.
      if (blobUrlRef.current) URL.revokeObjectURL(blobUrlRef.current);
      blobUrlRef.current = url;
      setBlobUrl(url);
    } catch (e) {
      if (thisGen !== loadGenRef.current) return;
      setError(
        typeof e === "string" ? (e as AdminAttachmentErrorCode) : String(e),
      );
    } finally {
      if (thisGen === loadGenRef.current) setLoading(false);
    }
  }, [
    origin,
    pubkey,
    feedbackId,
    attachment.sha256,
    attachment.mime,
    attachment.size,
  ]);

  // Auto-load image/* attachments immediately on mount — no button click needed.
  // Routes through the same `load` callback (generation fence, SSRF guard,
  // revoke-on-cleanup), so the existing blob-leak tests remain valid and cover
  // the auto-load path.
  // biome-ignore lint/correctness/useExhaustiveDependencies: load is a stable useCallback; attachment.mime is a mount-time constant — auto-load fires once per mount
  useEffect(() => {
    if (attachment.mime.startsWith("image/")) {
      void load();
    }
  }, []); // Empty: fires once on mount; identity boundary and generation fence handle context changes.

  if (error) {
    const friendlyError: Record<string, string> = {
      admin_attachment_too_large: "Attachment exceeds the 10 MiB desktop cap.",
      admin_attachment_mime_mismatch:
        "Attachment MIME type does not match the imeta record.",
      admin_attachment_size_mismatch:
        "Attachment byte count does not match the imeta record.",
      admin_attachment_network_error: "Network error fetching attachment.",
    };
    return (
      <div className="flex items-center gap-1.5 text-xs text-destructive">
        <AlertCircle className="h-3.5 w-3.5" />
        {friendlyError[error] ?? `Error: ${error}`}
      </div>
    );
  }

  if (!blobUrl) {
    // For image/* types the load is triggered automatically on mount.
    // Show only a spinner while in-flight; the "View attachment" button is
    // for non-image MIME types where the user opts in to loading.
    if (attachment.mime.startsWith("image/") || loading) {
      return (
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
          Loading…
        </div>
      );
    }
    return (
      <Button
        className="gap-1.5"
        disabled={loading}
        onClick={() => void load()}
        size="sm"
        type="button"
        variant="outline"
      >
        <Download className="h-3.5 w-3.5" />
        {`View attachment (${attachment.mime})`}
      </Button>
    );
  }

  if (attachment.mime.startsWith("image/")) {
    return (
      <img
        alt="Feedback attachment"
        className="max-h-96 max-w-full rounded-md border border-border/60 object-contain"
        src={blobUrl}
      />
    );
  }

  return (
    <a
      className="flex items-center gap-1.5 text-xs text-primary underline"
      download={`attachment-${attachment.sha256.slice(0, 8)}`}
      href={blobUrl}
    >
      <Download className="h-3.5 w-3.5" />
      Download attachment ({attachment.mime})
    </a>
  );
}

function FeedbackFields({ data }: { data: AdminFeedbackDto }) {
  return (
    <div
      className="space-y-1.5 rounded-md border border-border/60 px-3 py-2.5"
      data-testid="feedback-detail-fields"
    >
      <DetailRow label="ID" value={data.id} mono />
      <DetailRow label="Body" value={data.body} />
      <DetailRow label="Submitter" value={data.submitterPubkey} mono />
      <DetailRow label="Category" value={data.category ?? null} />
      <DetailRow label="Community" value={data.communityId} mono />
      <DetailRow label="Host" value={data.communityHost} />
      <DetailRow label="Event ID" value={data.eventId} mono />
      <DetailRow
        label="Event created"
        value={formatTimestamp(data.eventCreatedAt)}
      />
      <DetailRow label="Received" value={formatTimestamp(data.receivedAt)} />
    </div>
  );
}

function FeedbackDetail({
  origin,
  pubkey,
  generation,
  feedbackId,
  onBack,
}: {
  origin: string;
  pubkey: string;
  generation: number;
  feedbackId: string;
  onBack: () => void;
}) {
  const detailState = useAsyncLoad(
    () => getAdminFeedback(origin, feedbackId),
    [origin, pubkey, feedbackId],
    generation,
  );

  // Parse imeta attachment metadata from the relay's wire `tags: string[][]`.
  // AdminFeedback is serialised camelCase by the relay (serde rename_all).
  const attachments: AttachmentMeta[] =
    detailState.status === "ok"
      ? parseImetaAttachments(detailState.data.tags)
      : [];

  return (
    <div className="space-y-4">
      <button
        className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        onClick={onBack}
        type="button"
      >
        <ChevronLeft className="h-3.5 w-3.5" />
        Back to feedback
      </button>
      {detailState.status === "loading" && <LoadingSpinner />}
      {detailState.status === "error" && (
        <ErrorMessage message={detailState.message} />
      )}
      {detailState.status === "ok" && (
        <>
          <FeedbackFields data={detailState.data} />
          {attachments.length > 0 && (
            <div className="space-y-3">
              <h4 className="text-sm font-medium">Attachments</h4>
              {attachments.map((a) => (
                <AttachmentViewer
                  attachment={a}
                  feedbackId={feedbackId}
                  key={a.sha256}
                  origin={origin}
                  pubkey={pubkey}
                  panelGeneration={generation}
                />
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}

// ── Tab bar ───────────────────────────────────────────────────────────────

type Tab = "reports" | "feedback";

function TabBar({
  activeTab,
  onSelect,
}: {
  activeTab: Tab;
  onSelect: (tab: Tab) => void;
}) {
  return (
    <div className="mb-4 flex gap-1 border-b border-border/60">
      {(
        [
          { value: "reports" as const, label: "Reports", Icon: ShieldAlert },
          {
            value: "feedback" as const,
            label: "Feedback",
            Icon: MessageSquare,
          },
        ] as const
      ).map(({ value, label, Icon }) => (
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

// ── Shared helpers ────────────────────────────────────────────────────────

function LoadingSpinner() {
  return (
    <div className="flex items-center gap-2 text-sm text-muted-foreground">
      <LoaderCircle className="h-4 w-4 animate-spin" />
      Loading…
    </div>
  );
}

function ErrorMessage({ message }: { message: string }) {
  return (
    <div className="flex items-center gap-1.5 text-sm text-destructive">
      <AlertCircle className="h-4 w-4" />
      {message}
    </div>
  );
}

// ── Panel root ────────────────────────────────────────────────────────────

export function AdminConsolePanel({
  origin,
  pubkey,
}: {
  origin: string;
  /** Active identity pubkey — all state is keyed on (pubkey, origin). */
  pubkey: string;
}) {
  const [activeTab, setActiveTab] = useState<Tab>("reports");
  // Increment whenever the (pubkey, origin) context changes to invalidate all
  // in-flight useAsyncLoad effects via their effect-local `active` flags.
  const generationRef = useRef(0);
  const [generation, setGeneration] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey and origin are reactive props — effect fires when either changes to bump the generation fence
  useEffect(() => {
    generationRef.current += 1;
    setGeneration(generationRef.current);
  }, [pubkey, origin]);

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="admin-console-panel"
    >
      <TabBar activeTab={activeTab} onSelect={setActiveTab} />
      {activeTab === "reports" && (
        <ReportsTab origin={origin} pubkey={pubkey} generation={generation} />
      )}
      {activeTab === "feedback" && (
        <FeedbackTab origin={origin} pubkey={pubkey} generation={generation} />
      )}
    </div>
  );
}
