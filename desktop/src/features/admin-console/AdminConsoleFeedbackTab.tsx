/**
 * Feedback tab — shows the deployment-wide product feedback queue with
 * optional image attachment viewer and status triage controls.
 *
 * The attachment viewer uses a per-load generation fence to discard results
 * from superseded loads on identity/origin change or unmount.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, ChevronLeft, Download, LoaderCircle } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  fetchAdminAttachmentBlobUrl,
  getAdminFeedback,
  listAdminFeedback,
  patchAdminFeedback,
  type AdminAttachmentErrorCode,
  type AdminFeedbackDto,
  type AdminFeedbackStatus,
  type AdminFeedbackSummaryDto,
} from "./api";
import {
  type AsyncState,
  type AttachmentMeta,
  adminErrorMessage,
  CommunityGroupedList,
  DetailRow,
  ErrorMessage,
  LoadingSpinner,
  formatTimestamp,
  parseImetaAttachments,
  useAsyncLoad,
} from "./AdminConsolePanelHelpers";

// ── Feedback tab ──────────────────────────────────────────────────────────

export function FeedbackTab({
  origin,
  pubkey,
  generation,
}: {
  origin: string;
  pubkey: string;
  generation: number;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // List refresh fence: bumped when a feedback status change completes in the
  // detail view, so returning to the list shows fresh status without a tab
  // switch.
  const [listGen, setListGen] = useState(0);

  const listState: AsyncState<AdminFeedbackSummaryDto[]> = useAsyncLoad(
    () => listAdminFeedback(origin),
    [origin, pubkey],
    generation + listGen,
  );

  if (selectedId) {
    return (
      <FeedbackDetail
        feedbackId={selectedId}
        onBack={() => setSelectedId(null)}
        origin={origin}
        pubkey={pubkey}
        generation={generation}
        onMutated={() => setListGen((g) => g + 1)}
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
    <CommunityGroupedList
      items={items}
      renderItem={(item: AdminFeedbackSummaryDto) => {
        const id = item.id;
        const text = item.bodySummary.slice(0, 120);
        const receivedAt = item.receivedAt;
        const status = item.status;
        return (
          <li key={id}>
            <button
              className="w-full rounded-md border border-border/60 px-3 py-2.5 text-left text-sm hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => setSelectedId(id)}
              type="button"
            >
              <div className="flex items-center gap-2 mb-0.5">
                <span className="block font-medium line-clamp-2">{text}</span>
                {status !== "new" && (
                  <Badge className="shrink-0" variant="secondary">
                    {status}
                  </Badge>
                )}
              </div>
              {receivedAt && (
                <span className="text-xs text-muted-foreground">
                  {formatTimestamp(receivedAt)}
                </span>
              )}
            </button>
          </li>
        );
      }}
    />
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

// ── Feedback fields ───────────────────────────────────────────────────────

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

// ── Feedback status control ───────────────────────────────────────────────

/**
 * Status-control widget for a feedback entry.
 * Lets operators/moderators triage feedback by marking it as
 * `reviewed` or `archived` (or reverting to `new`).
 */
function FeedbackStatusControl({
  feedbackId,
  currentStatus,
  origin,
  onStatusChanged,
}: {
  feedbackId: string;
  currentStatus: AdminFeedbackStatus | null;
  origin: string;
  onStatusChanged: (newStatus: AdminFeedbackStatus) => void;
}) {
  const [isWorking, setIsWorking] = useState(false);

  const statuses: AdminFeedbackStatus[] = ["new", "reviewed", "archived"];

  const handleStatusChange = async (newStatus: AdminFeedbackStatus) => {
    if (newStatus === currentStatus) return;
    setIsWorking(true);
    try {
      await patchAdminFeedback(origin, feedbackId, newStatus);
      toast.success(`Feedback marked ${newStatus}`);
      onStatusChanged(newStatus);
    } catch (e) {
      toast.error(adminErrorMessage(e));
    } finally {
      setIsWorking(false);
    }
  };

  return (
    <div
      className="flex flex-col gap-1.5"
      data-testid="feedback-status-control"
    >
      <span className="text-xs font-medium text-muted-foreground">Status</span>
      <div className="flex gap-1.5">
        {statuses.map((s) => (
          <Button
            className={cn(
              "text-xs",
              currentStatus === s && "ring-2 ring-ring ring-offset-1",
            )}
            data-testid={`feedback-status-btn-${s}`}
            disabled={isWorking}
            key={s}
            onClick={() => void handleStatusChange(s)}
            size="sm"
            type="button"
            variant={currentStatus === s ? "default" : "outline"}
          >
            {s}
          </Button>
        ))}
      </div>
    </div>
  );
}

// ── Feedback detail ───────────────────────────────────────────────────────

export function FeedbackDetail({
  origin,
  pubkey,
  generation,
  feedbackId,
  onBack,
  onMutated,
}: {
  origin: string;
  pubkey: string;
  generation: number;
  feedbackId: string;
  onBack: () => void;
  /** Called after a status change completes so the parent list can refetch. */
  onMutated: () => void;
}) {
  // Local status state: initialized from server, updated on PATCH.
  const [localStatus, setLocalStatus] = useState<AdminFeedbackStatus | null>(
    null,
  );

  const detailState: AsyncState<AdminFeedbackDto> = useAsyncLoad(
    () => getAdminFeedback(origin, feedbackId),
    [origin, pubkey, feedbackId],
    generation,
  );

  // Sync localStatus from server data on load (but not on every re-render).
  // `status` is a required wire field — read it directly, never default a
  // missing value to "new" (that would misreport a reviewed/archived entry).
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — sync once when data arrives
  useEffect(() => {
    if (detailState.status === "ok") {
      setLocalStatus(detailState.data.status);
    }
  }, [detailState.status === "ok"]);

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
          <FeedbackStatusControl
            feedbackId={feedbackId}
            currentStatus={localStatus}
            origin={origin}
            onStatusChanged={(newStatus) => {
              setLocalStatus(newStatus);
              onMutated();
            }}
          />
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
