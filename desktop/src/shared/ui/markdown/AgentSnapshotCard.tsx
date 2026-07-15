import * as React from "react";
import { Bot, Download, Loader2, Users } from "lucide-react";

import { invokeTauri } from "@/shared/api/tauri";
import { fetchSnapshotBytes } from "@/shared/api/tauriMedia";
import { cn } from "@/shared/lib/cn";
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";

export type AgentSnapshotCardProps = {
  displayName: string;
  href: string;
  filename: string;
  sharedBy?: string;
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
 * Snapshot attachment card rendered in a message timeline when an agent or
 * team snapshot attachment is classified as an importable snapshot candidate.
 *
 * Shows two independent actions:
 * - **Add agent/team** — bounded, verified in-memory fetch → existing importer
 * - **Download** — native save dialog via the unchanged `download_file` command
 *
 * The display name and sharer are presentation-only message metadata. Import
 * still fetches and verifies the snapshot bytes before the existing preview
 * flow presents the decoded snapshot details.
 */
export function AgentSnapshotCard({
  displayName,
  href,
  filename,
  sharedBy,
  size,
  sha256,
  snapshotKind,
  thumb,
  onImport,
}: AgentSnapshotCardProps) {
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
          err instanceof Error
            ? err.message
            : `Couldn’t load this ${snapshotKind}. Try again.`,
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
  const SnapshotIcon = snapshotKind === "team" ? Users : Bot;
  const showThumb = !!thumb && !thumbError;
  const formattedSize =
    size == null
      ? null
      : size < 1024
        ? `${size} B`
        : size < 1024 * 1024
          ? `${(size / 1024).toFixed(1)} KB`
          : `${(size / (1024 * 1024)).toFixed(1)} MB`;
  const metadata = [sharedBy ? `Shared by ${sharedBy}` : null, formattedSize]
    .filter(Boolean)
    .join(" · ");

  return (
    <Attachment
      className="my-1 inline-flex w-fit max-w-full shadow-none"
      data-testid="agent-snapshot-card"
      state={importState.phase === "error" ? "error" : "done"}
    >
      <AttachmentMedia
        className={cn(
          showThumb
            ? "relative h-9 w-9"
            : "bg-primary/10 text-primary ring-1 ring-primary/20 dark:bg-primary/15",
        )}
        variant={showThumb ? "image" : "icon"}
      >
        {showThumb ? (
          <>
            <img
              alt=""
              aria-hidden="true"
              className="pointer-events-none absolute inset-0 h-full w-full scale-150 object-cover"
              src={thumb}
              referrerPolicy="no-referrer"
            />
            <img
              alt=""
              className="relative h-full w-full object-cover"
              data-testid="agent-snapshot-card-thumb"
              src={thumb}
              referrerPolicy="no-referrer"
              onError={() => setThumbError(true)}
            />
          </>
        ) : (
          <SnapshotIcon />
        )}
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle
          className="overflow-visible whitespace-normal text-clip"
          title={displayName}
        >
          {displayName}
        </AttachmentTitle>
        {importState.phase === "error" ? (
          <AttachmentDescription
            className="overflow-visible whitespace-normal text-clip text-destructive"
            data-testid="agent-snapshot-card-error"
          >
            {importState.message}
          </AttachmentDescription>
        ) : metadata ? (
          <AttachmentDescription className="overflow-visible whitespace-normal text-clip text-secondary-foreground/75">
            {metadata}
          </AttachmentDescription>
        ) : null}
      </AttachmentContent>
      <AttachmentActions
        aria-label={`Actions for ${displayName}`}
        className="ml-4 gap-2"
        role="group"
      >
        <AttachmentAction
          aria-label={`Download ${displayName}`}
          data-testid="agent-snapshot-card-download"
          onClick={handleDownload}
          size="icon"
          title="Download"
          type="button"
          variant="ghost"
        >
          <Download />
        </AttachmentAction>
        <AttachmentAction
          className="text-primary-foreground shadow-none hover:bg-primary/90 hover:text-primary-foreground hover:shadow-none"
          data-testid="agent-snapshot-card-import"
          disabled={isFetching}
          onClick={handleImport}
          size="sm"
          type="button"
          variant="default"
        >
          {isFetching ? <Loader2 className="animate-spin" /> : <SnapshotIcon />}
          {isFetching
            ? "Loading…"
            : snapshotKind === "team"
              ? "Add team"
              : "Add agent"}
        </AttachmentAction>
      </AttachmentActions>
    </Attachment>
  );
}
