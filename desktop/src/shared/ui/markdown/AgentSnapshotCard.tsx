import * as React from "react";
import { AlertCircle, Bot, Download, Loader2 } from "lucide-react";

import { invokeTauri } from "@/shared/api/tauri";
import { fetchSnapshotBytes } from "@/shared/api/tauriMedia";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

export type AgentSnapshotCardProps = {
  href: string;
  filename: string;
  size?: number;
  sha256: string;
  /** Discriminant used to label the card and route the import. */
  snapshotKind: "agent" | "team";
  /**
   * Optional thumbnail URL for the card icon — the agent's avatar image.
   * When present, renders in place of the generic Bot icon. Falls back to
   * the Bot icon when absent, when the URL is a non-image MIME, or when
   * the image fails to load.
   */
  thumb?: string;
  /**
   * Called after bytes are successfully fetched and decoded. The card
   * navigates to /agents and triggers the existing importer flow via this
   * callback. The caller (markdown renderer) must supply the app-level
   * navigation + pending-import wiring.
   */
  onImport: (fileBytes: number[], fileName: string) => void;
};

type ImportState =
  | { phase: "idle" }
  | { phase: "fetching" }
  | { phase: "error"; message: string };

/**
 * Snapshot attachment card rendered in a message timeline when an `.agent.json`
 * or `.agent.png` attachment is classified as an agent snapshot candidate.
 *
 * Shows two independent actions:
 * - **Import agent** — bounded, verified in-memory fetch → existing importer
 * - **Download** — native save dialog via the unchanged `download_file` command
 *
 * The card text identifies the attachment as "Agent snapshot" (untrusted label)
 * until the bytes have been verified by the Rust decoder; the sender-supplied
 * agent name is never shown here.
 */
export function AgentSnapshotCard({
  href,
  filename,
  size,
  sha256,
  snapshotKind,
  thumb,
  onImport,
}: AgentSnapshotCardProps) {
  const cardRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(cardRef);

  const [importState, setImportState] = React.useState<ImportState>({
    phase: "idle",
  });
  const inFlightRef = React.useRef(false);
  const [thumbError, setThumbError] = React.useState(false);

  async function handleImport() {
    if (inFlightRef.current) return; // prevent double-click
    inFlightRef.current = true;
    setImportState({ phase: "fetching" });
    try {
      const fileBytes = await fetchSnapshotBytes({
        url: href,
        filename,
        expectedSha256: sha256,
        expectedSize: size ?? 0,
      });
      setImportState({ phase: "idle" });
      onImport(fileBytes, filename);
    } catch (err) {
      setImportState({
        phase: "error",
        message:
          err instanceof Error ? err.message : "Failed to fetch snapshot.",
      });
    } finally {
      inFlightRef.current = false;
    }
  }

  function handleDownload() {
    invokeTauri("download_file", { url: href, filename }).catch(() => {
      /* download errors are surfaced by the Rust side via toast */
    });
  }

  const isFetching = importState.phase === "fetching";
  const showThumb = !!thumb && !thumbError;

  return (
    <div
      ref={cardRef}
      className="my-1 inline-flex max-w-sm flex-col gap-2 rounded-2xl border border-primary/25 bg-primary/5 px-3 py-2 text-left"
      style={{ borderRadius: "1rem" }}
      data-testid="agent-snapshot-card"
    >
      {/* Header row: icon + filename */}
      <div className="flex items-center gap-2">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary overflow-hidden">
          {showThumb ? (
            <img
              alt=""
              className="h-full w-full object-cover"
              data-testid="agent-snapshot-card-thumb"
              src={thumb}
              referrerPolicy="no-referrer"
              onError={() => setThumbError(true)}
            />
          ) : (
            <Bot className="h-4 w-4" />
          )}
        </span>
        <div className="min-w-0 flex-1">
          <span
            className="block truncate text-sm font-medium text-foreground"
            title={filename}
          >
            {filename}
          </span>
          <span className="block text-xs text-muted-foreground">
            {snapshotKind === "team" ? "Team snapshot" : "Agent snapshot"}
            {size != null
              ? ` · ${size < 1024 ? `${size} B` : size < 1024 * 1024 ? `${(size / 1024).toFixed(1)} KB` : `${(size / (1024 * 1024)).toFixed(1)} MB`}`
              : ""}
          </span>
        </div>
      </div>

      {/* Error row */}
      {importState.phase === "error" && (
        <div
          className="flex items-start gap-1.5 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-xs text-destructive"
          data-testid="agent-snapshot-card-error"
        >
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{importState.message}</span>
        </div>
      )}

      {/* Action row */}
      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={isFetching}
          onClick={handleImport}
          className="flex h-7 flex-1 items-center justify-center gap-1.5 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
          data-testid="agent-snapshot-card-import"
        >
          {isFetching ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Bot className="h-3.5 w-3.5" />
          )}
          {isFetching
            ? "Fetching…"
            : snapshotKind === "team"
              ? "Import team"
              : "Import agent"}
        </button>
        <button
          type="button"
          onClick={handleDownload}
          className="flex h-7 items-center justify-center gap-1 rounded-lg border border-border bg-background px-3 text-xs font-medium text-foreground transition-colors hover:bg-muted"
          data-testid="agent-snapshot-card-download"
        >
          <Download className="h-3 w-3" />
          Download
        </button>
      </div>
    </div>
  );
}
