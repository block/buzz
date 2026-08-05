import * as React from "react";
import { EditorContent } from "@tiptap/react";
import { Eye, FileCode2, TriangleAlert } from "lucide-react";

import type { DocumentTab } from "@/features/documents/lib/documentTabs";
import { useVaultEditor } from "@/features/documents/lib/editor/useVaultEditor";
import type { WikilinkClickHandler } from "@/features/documents/lib/editor/wikilinkExtension";
import type { NoteIndex } from "@/features/documents/lib/noteIndex";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

/**
 * Warns that live preview would reformat this file.
 *
 * This is the visible half of the round-trip guard: the note opens in source
 * mode, and switching to live preview is an explicit choice rather than a
 * silent default.
 */
function RoundTripBanner({ onSwitchToLive }: { onSwitchToLive: () => void }) {
  return (
    <div
      className="flex items-start gap-2 border-b border-border/60 bg-amber-500/10 px-4 py-2 text-sm"
      data-testid="documents-round-trip-banner"
    >
      <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-500" />
      <div className="min-w-0 flex-1">
        <p>
          Live preview would reformat this file, so it opened in source mode.
        </p>
        <p className="mt-0.5 text-muted-foreground">
          Tables, callouts, footnotes and raw HTML are not yet represented in
          the editor.
        </p>
      </div>
      <Button
        className="shrink-0"
        data-testid="documents-round-trip-switch"
        onClick={onSwitchToLive}
        size="sm"
        type="button"
        variant="ghost"
      >
        Use live preview anyway
      </Button>
    </div>
  );
}

/** Offers a choice when a dirty file changed underneath the user. */
function ExternalChangeBanner({
  onKeepMine,
  onReload,
}: {
  onKeepMine: () => void;
  onReload: () => void;
}) {
  return (
    <div
      className="flex items-center gap-2 border-b border-border/60 bg-sky-500/10 px-4 py-2 text-sm"
      data-testid="documents-external-change-banner"
    >
      <TriangleAlert className="h-4 w-4 shrink-0 text-sky-600 dark:text-sky-400" />
      <p className="min-w-0 flex-1">
        This file changed on disk while you had unsaved edits.
      </p>
      <Button
        data-testid="documents-external-reload"
        onClick={onReload}
        size="sm"
        type="button"
        variant="ghost"
      >
        Reload from disk
      </Button>
      <Button
        data-testid="documents-external-keep"
        onClick={onKeepMine}
        size="sm"
        type="button"
        variant="ghost"
      >
        Keep my version
      </Button>
    </div>
  );
}

/**
 * Source mode: a plain textarea over the note body.
 *
 * Deliberately never touches the markdown serializer — what the user types is
 * exactly what is written. This is the escape hatch that makes the round-trip
 * guard acceptable rather than merely restrictive.
 */
function DocumentSourceEditor({
  onChange,
  onSave,
  tab,
}: {
  onChange: (markdown: string) => void;
  onSave: () => void;
  tab: DocumentTab;
}) {
  return (
    <textarea
      className="min-h-0 flex-1 resize-none bg-transparent p-6 font-mono text-sm leading-relaxed outline-none"
      data-testid="documents-source-editor"
      onChange={(event) => onChange(event.target.value)}
      onKeyDown={(event) => {
        if (
          (event.metaKey || event.ctrlKey) &&
          event.key.toLowerCase() === "s"
        ) {
          event.preventDefault();
          onSave();
        }
      }}
      spellCheck={false}
      value={tab.content}
    />
  );
}

function DocumentLiveEditor({
  noteIndex,
  onChange,
  onSave,
  onWikilinkClick,
  tab,
}: {
  noteIndex: NoteIndex | null;
  onChange: (markdown: string) => void;
  onSave: () => void;
  onWikilinkClick: WikilinkClickHandler;
  tab: DocumentTab;
}) {
  /**
   * The last content this editor itself produced.
   *
   * `tab.content` changes for two very different reasons: the user typing (the
   * editor is already showing it — reloading would reset the cursor) and an
   * external reload from disk (the editor is stale and must be refreshed).
   * Recording what we emitted distinguishes them; comparing paths alone misses
   * the second case entirely.
   */
  const lastEmittedRef = React.useRef<string | null>(null);

  const handleChange = React.useCallback(
    (markdown: string) => {
      lastEmittedRef.current = markdown;
      onChange(markdown);
    },
    [onChange],
  );

  const { editor, loadDocument } = useVaultEditor({
    currentPath: tab.path,
    noteIndex,
    onChange: handleChange,
    onSave,
    onWikilinkClick,
  });

  React.useEffect(() => {
    if (!editor) return;
    if (lastEmittedRef.current === tab.content) return;
    lastEmittedRef.current = tab.content;
    loadDocument(tab.content);
  }, [editor, loadDocument, tab.content]);

  return (
    <EditorContent
      className="min-h-0 flex-1 overflow-y-auto px-6 py-4"
      data-testid="documents-live-editor"
      editor={editor}
    />
  );
}

export function DocumentEditorPane({
  hasExternalChange,
  noteIndex,
  onChange,
  onKeepMine,
  onReload,
  onSave,
  onSetViewMode,
  onWikilinkClick,
  tab,
}: {
  hasExternalChange: boolean;
  noteIndex: NoteIndex | null;
  onChange: (markdown: string) => void;
  onKeepMine: () => void;
  onReload: () => void;
  onSave: () => void;
  onSetViewMode: (mode: "live" | "source") => void;
  onWikilinkClick: WikilinkClickHandler;
  tab: DocumentTab;
}) {
  const isSource = tab.viewMode === "source";

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {hasExternalChange ? (
        <ExternalChangeBanner onKeepMine={onKeepMine} onReload={onReload} />
      ) : null}
      {tab.roundTrip === "lossy" && isSource ? (
        <RoundTripBanner onSwitchToLive={() => onSetViewMode("live")} />
      ) : null}

      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-border/60 px-4 py-1.5">
        <span className="truncate text-sm text-muted-foreground">
          {tab.name}
        </span>
        <Button
          className={cn("shrink-0", isSource && "text-foreground")}
          data-testid="documents-toggle-view-mode"
          onClick={() => onSetViewMode(isSource ? "live" : "source")}
          size="sm"
          type="button"
          variant="ghost"
        >
          {isSource ? (
            <>
              <Eye className="h-4 w-4" />
              Live preview
            </>
          ) : (
            <>
              <FileCode2 className="h-4 w-4" />
              Source
            </>
          )}
        </Button>
      </div>

      {isSource ? (
        <DocumentSourceEditor onChange={onChange} onSave={onSave} tab={tab} />
      ) : (
        // Remounting per path keeps the editor's history from leaking across
        // files — undo must never resurrect a different note's text.
        <DocumentLiveEditor
          key={tab.path}
          noteIndex={noteIndex}
          onChange={onChange}
          onSave={onSave}
          onWikilinkClick={onWikilinkClick}
          tab={tab}
        />
      )}
    </div>
  );
}
