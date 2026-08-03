import {
  FileIcon,
  ImageIcon,
  VideoIcon,
  Download,
  Copy,
  ArrowRight,
  ShieldCheck,
} from "lucide-react";
import { toast } from "sonner";
import type { ChannelFile } from "./useChannelFiles";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function formatSize(bytes: number | undefined): string {
  if (bytes == null || bytes === 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatDate(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));
  if (diffDays === 0) return "Today";
  if (diffDays === 1) return "Yesterday";
  if (diffDays < 7) return `${diffDays}d ago`;
  if (diffDays < 30) return `${Math.floor(diffDays / 7)}w ago`;
  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function fileTypeLabel(mimeType: string): string {
  if (mimeType.startsWith("image/")) return "Image";
  if (mimeType.startsWith("video/")) return "Video";
  if (mimeType.startsWith("audio/")) return "Audio";
  if (mimeType.includes("pdf")) return "PDF";
  if (mimeType.includes("zip") || mimeType.includes("tar")) return "Archive";
  if (
    mimeType.includes("text/") ||
    mimeType.includes("json") ||
    mimeType.includes("xml")
  )
    return "Text";
  return "File";
}

async function copyToClipboard(text: string, label: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(`${label} copied`);
  } catch {
    toast.error("Failed to copy");
  }
}

function FileRowThumbnail({ file }: { file: ChannelFile }) {
  if (file.mimeType.startsWith("image/")) {
    return (
      <div className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted">
        <img
          alt=""
          className="h-full w-full object-cover"
          loading="lazy"
          src={file.url}
        />
      </div>
    );
  }
  if (file.mimeType.startsWith("video/")) {
    return (
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted">
        <VideoIcon className="h-5 w-5 text-muted-foreground" />
      </div>
    );
  }
  return (
    <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted">
      <FileIcon className="h-5 w-5 text-muted-foreground" />
    </div>
  );
}

export type FileRowProps = {
  file: ChannelFile;
  senderName?: string;
  senderAvatarUrl?: string | null;
  onJumpToMessage?: (eventId: string) => void;
  onDragStart?: (e: React.DragEvent, eventId: string) => void;
  /** Selection mode */
  selecting?: boolean;
  selected?: boolean;
  onToggleSelect?: (eventId: string) => void;
};

export function FileRow({
  file,
  senderName,
  senderAvatarUrl,
  onJumpToMessage,
  onDragStart,
  selecting,
  selected,
  onToggleSelect,
}: FileRowProps) {
  const filename = file.filename ?? file.rawUrl.split("/").pop() ?? "file";

  const handleClick = (e: React.MouseEvent) => {
    if (selecting) {
      e.preventDefault();
      onToggleSelect?.(file.eventId);
    }
  };

  const handleCheckboxClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    onToggleSelect?.(file.eventId);
  };

  return (
    <div
      className={`group flex items-center gap-3 rounded-lg px-3 py-2.5 transition-colors ${
        selected ? "bg-primary/5" : "hover:bg-muted/50"
      } ${selecting ? "cursor-pointer" : ""}`}
      draggable={!!onDragStart}
      onClick={handleClick}
      onDragStart={(e) => onDragStart?.(e, file.eventId)}
    >
      {selecting ? (
        <div className="flex shrink-0 items-center" onClick={handleCheckboxClick}>
          <div
            className={`flex h-5 w-5 items-center justify-center rounded border-2 transition-colors ${
              selected
                ? "border-primary bg-primary text-primary-foreground"
                : "border-muted-foreground/30 hover:border-muted-foreground/50"
            }`}
            role="checkbox"
            aria-checked={selected}
            aria-label={`Select ${filename}`}
          >
            {selected ? (
              <svg className="h-3 w-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                <path d="M5 13l4 4L19 7" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            ) : null}
          </div>
        </div>
      ) : null}

      <a
        className="contents"
        download={filename}
        href={file.url}
        rel="noreferrer"
        target="_blank"
        onClick={(e) => { if (selecting) e.preventDefault(); }}
      >
        <FileRowThumbnail file={file} />
      </a>

      <div className="min-w-0 flex-1">
        <a
          className="truncate text-sm font-medium hover:underline"
          download={filename}
          href={file.url}
          rel="noreferrer"
          target="_blank"
          onClick={(e) => { if (selecting) e.preventDefault(); }}
          title={filename}
        >
          {filename}
        </a>
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span>{fileTypeLabel(file.mimeType)}</span>
          {file.dim ? (
            <>
              <span aria-hidden="true">·</span>
              <span>{file.dim}</span>
            </>
          ) : null}
          {file.size != null ? (
            <>
              <span aria-hidden="true">·</span>
              <span>{formatSize(file.size)}</span>
            </>
          ) : null}
          {file.sha256 ? (
            <span
              className="inline-flex items-center gap-0.5 text-green-600 dark:text-green-400"
              title="SHA-256 verified"
            >
              <ShieldCheck className="h-3 w-3" />
            </span>
          ) : null}
        </p>
        {file.caption ? (
          <p className="mt-0.5 truncate text-xs text-muted-foreground/70">
            {file.caption}
          </p>
        ) : null}
      </div>

      <div className="flex shrink-0 items-center gap-3 text-xs text-muted-foreground">
        <div className="flex items-center gap-1.5">
          <UserAvatar
            avatarUrl={senderAvatarUrl ?? null}
            className="h-5 w-5"
            displayName={senderName ?? file.pubkey.slice(0, 8)}
            size="xs"
          />
          {senderName ? (
            <span className="hidden max-w-[100px] truncate sm:inline">
              {senderName}
            </span>
          ) : null}
        </div>
        <span className="w-16 text-right">{formatDate(file.createdAt)}</span>

        <div className="flex items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
            <button
              aria-label={`Copy link for ${filename}`}
              className="rounded p-1 hover:bg-muted"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                void copyToClipboard(file.url, "Link");
              }}
              type="button"
            >
              <Copy className="h-3.5 w-3.5" />
            </button>
            <a
              aria-label={`Download ${filename}`}
              className="rounded p-1 hover:bg-muted"
              download={filename}
              href={file.url}
              onClick={(e) => e.stopPropagation()}
            >
              <Download className="h-3.5 w-3.5" />
            </a>
            {onJumpToMessage ? (
              <button
                aria-label="Jump to message"
                className="rounded p-1 hover:bg-muted"
                onClick={(e) => {
                  e.stopPropagation();
                  onJumpToMessage(file.eventId);
                }}
                type="button"
              >
                <ArrowRight className="h-3.5 w-3.5" />
              </button>
            ) : null}
          </div>
      </div>
    </div>
  );
}

export function FileRowSkeleton() {
  return (
    <div className="flex animate-pulse items-center gap-3 rounded-lg px-3 py-2.5">
      <div className="h-10 w-10 shrink-0 rounded-md bg-muted" />
      <div className="flex-1 space-y-1.5">
        <div className="h-4 w-48 rounded bg-muted" />
        <div className="h-3 w-24 rounded bg-muted" />
      </div>
      <div className="h-5 w-5 rounded-full bg-muted" />
      <div className="h-3 w-16 rounded bg-muted" />
    </div>
  );
}
