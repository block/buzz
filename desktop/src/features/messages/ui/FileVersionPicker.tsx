import * as React from "react";
import { Link2, Search } from "lucide-react";

import {
  type ChannelFileEntry,
  listChannelFiles,
} from "@/shared/api/channelFiles";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

export type FileVersionPickerExclusion = {
  /** Exclude the candidate sharing this sha256 — a file can't supersede a
   * byte-identical copy of itself. */
  sha256?: string | null;
  /** Exclude the candidate with this event id — a file can't supersede
   * itself. */
  eventId?: string | null;
};

type FileVersionPickerProps = {
  channelId: string;
  exclude?: FileVersionPickerExclusion;
  onOpenChange: (open: boolean) => void;
  onSelect: (file: ChannelFileEntry) => void;
  open: boolean;
  trigger: React.ReactNode;
};

/**
 * Small searchable popover listing a channel's current (non-superseded)
 * files, for manually linking something as a "new version of" one of them.
 * Shared by two entry points:
 *
 *   - the composer's per-attachment "Link to a different file…" affordance
 *     (`ComposerAttachments.tsx`), picking an earlier upload for an
 *     attachment that hasn't been sent yet;
 *   - the Files tab's per-row retroactive-link action (`FilesPanel.tsx`),
 *     picking an earlier upload for a file that was already sent.
 *
 * Reuses the same `Popover` primitive as `ComposerEmojiPicker` and the
 * mention/channel autocompletes rather than inventing new popover machinery.
 * Fetches `listChannelFiles` lazily on open (not on mount), since the full
 * page-through-history list can be sizeable and most attachments never open
 * the picker.
 */
export function FileVersionPicker({
  channelId,
  exclude,
  onOpenChange,
  onSelect,
  open,
  trigger,
}: FileVersionPickerProps) {
  const [query, setQuery] = React.useState("");
  const [files, setFiles] = React.useState<ChannelFileEntry[] | null>(null);
  const [loadError, setLoadError] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    setQuery("");
    setFiles(null);
    setLoadError(false);
    let cancelled = false;
    void (async () => {
      try {
        const result = await listChannelFiles(channelId);
        if (!cancelled) setFiles(result);
      } catch {
        if (!cancelled) setLoadError(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, channelId]);

  const candidates = React.useMemo(() => {
    if (!files) return [];
    const normalizedQuery = query.trim().toLowerCase();
    return files.filter((file) => {
      if (file.filename == null) return false;
      if (file.supersededBy != null) return false; // already outdated
      if (exclude?.eventId && file.eventId === exclude.eventId) return false;
      if (exclude?.sha256 && file.sha256 && file.sha256 === exclude.sha256) {
        return false;
      }
      if (!normalizedQuery) return true;
      return file.filename.toLowerCase().includes(normalizedQuery);
    });
  }, [files, query, exclude?.eventId, exclude?.sha256]);

  return (
    <Popover onOpenChange={onOpenChange} open={open}>
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-64 p-2"
        data-testid="file-version-picker"
        side="top"
        sideOffset={8}
      >
        <div className="flex items-center gap-1.5 rounded-lg border border-border/60 bg-background px-2 py-1">
          <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <input
            // biome-ignore lint/a11y/noAutofocus: popover search
            autoFocus
            className="w-full bg-transparent text-xs outline-hidden placeholder:text-muted-foreground"
            data-testid="file-version-picker-search"
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search files…"
            value={query}
          />
        </div>
        <div className="mt-1.5 max-h-48 overflow-y-auto">
          {files === null && !loadError ? (
            <div className="px-2 py-1.5 text-2xs text-muted-foreground">
              Loading…
            </div>
          ) : loadError ? (
            <div className="px-2 py-1.5 text-2xs text-muted-foreground">
              Couldn't load this channel's files.
            </div>
          ) : candidates.length === 0 ? (
            <div className="px-2 py-1.5 text-2xs text-muted-foreground">
              No matching files.
            </div>
          ) : (
            candidates.map((file) => (
              <button
                className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-popover-foreground hover:bg-accent/50"
                data-testid="file-version-picker-option"
                key={file.eventId}
                onMouseDown={(event) => {
                  event.preventDefault();
                  onSelect(file);
                  onOpenChange(false);
                }}
                type="button"
              >
                <Link2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate">{file.filename}</span>
              </button>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
