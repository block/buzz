import * as React from "react";
import { EditorContent } from "@tiptap/react";
import { Eye, FileCode2, TriangleAlert } from "lucide-react";

import type { DocumentTab } from "@/features/documents/lib/documentTabs";
import { useVaultEditor } from "@/features/documents/lib/editor/useVaultEditor";
import type { WikilinkClickHandler } from "@/features/documents/lib/editor/wikilinkExtension";
import type { NoteIndex } from "@/features/documents/lib/noteIndex";
import { useAlwaysLivePreview } from "@/features/documents/useDocumentsPreferences";
import {
  activeHeadingIndex,
  type OutlineHeading,
} from "@/features/documents/lib/obsidianSyntax";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

/**
 * Notes that this file will be reformatted if edited in live preview.
 *
 * Deliberately one short line with no action button: the mode toggle sits
 * immediately below in the header, and having both said "live preview" read as
 * two competing controls for the same thing.
 */
function RoundTripNotice() {
  return (
    <p
      className="flex items-center gap-1.5 border-b border-border/60 bg-amber-500/10 px-4 py-1.5 text-2xs text-amber-700 dark:text-amber-500"
      data-testid="documents-round-trip-banner"
    >
      <TriangleAlert className="h-3.5 w-3.5 shrink-0" />
      Live preview would reformat this file — it uses markdown the editor does
      not yet support.
    </p>
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
  headings,
  noteIndex,
  onActiveHeadingChange,
  onChange,
  onHeadingsChange,
  onRegisterScroll,
  onSave,
  onWikilinkClick,
  tab,
}: {
  noteIndex: NoteIndex | null;
  onChange: (markdown: string) => void;
  onActiveHeadingChange: (index: number) => void;
  onHeadingsChange: (headings: OutlineHeading[]) => void;
  onRegisterScroll: (scroll: ((position: number) => void) | null) => void;
  onSave: () => void;
  onWikilinkClick: WikilinkClickHandler;
  tab: DocumentTab;
  headings: readonly OutlineHeading[];
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

  const { editor, loadDocument, measureOffsets, scrollToPosition } =
    useVaultEditor({
      currentPath: tab.path,
      noteIndex,
      onChange: handleChange,
      onHeadingsChange,
      onSave,
      onWikilinkClick,
    });

  // Hand the scroll function up so the outline panel (a sibling of this
  // component) can jump to a heading.
  React.useEffect(() => {
    onRegisterScroll(editor ? scrollToPosition : null);
    return () => onRegisterScroll(null);
  }, [editor, onRegisterScroll, scrollToPosition]);

  // Scroll-spy: recompute which heading is active as the editor scrolls.
  React.useEffect(() => {
    const container = editor?.view.dom.parentElement;
    if (!container || headings.length === 0) {
      onActiveHeadingChange(-1);
      return;
    }
    const update = () => {
      const offsets = measureOffsets(
        headings.map((heading) => heading.position),
      );
      onActiveHeadingChange(activeHeadingIndex(offsets, container.scrollTop));
    };
    update();
    container.addEventListener("scroll", update, { passive: true });
    return () => container.removeEventListener("scroll", update);
  }, [editor, headings, measureOffsets, onActiveHeadingChange]);

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
  headings,
  noteIndex,
  onActiveHeadingChange,
  onChange,
  onHeadingsChange,
  onKeepMine,
  onRegisterScroll,
  onReload,
  onSave,
  onSetViewMode,
  onWikilinkClick,
  tab,
}: {
  hasExternalChange: boolean;
  noteIndex: NoteIndex | null;
  onActiveHeadingChange: (index: number) => void;
  onChange: (markdown: string) => void;
  onHeadingsChange: (headings: OutlineHeading[]) => void;
  onKeepMine: () => void;
  onRegisterScroll: (scroll: ((position: number) => void) | null) => void;
  headings: readonly OutlineHeading[];
  onReload: () => void;
  onSave: () => void;
  onSetViewMode: (mode: "live" | "source") => void;
  onWikilinkClick: WikilinkClickHandler;
  tab: DocumentTab;
}) {
  const isSource = tab.viewMode === "source";
  // With the setting on, the user has already accepted that these files get
  // reformatted; repeating the warning on every note is just noise.
  const alwaysLivePreview = useAlwaysLivePreview();

  // The outline is published by the live editor's plugin, so a note opened in
  // source mode leaves whatever the *previous* note published standing — an
  // outline of a file the user is no longer looking at, complete with working
  // scroll-to targets into a document that is no longer mounted. Clear it.
  React.useEffect(() => {
    if (isSource) onHeadingsChange([]);
  }, [isSource, onHeadingsChange, tab.path]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {hasExternalChange ? (
        <ExternalChangeBanner onKeepMine={onKeepMine} onReload={onReload} />
      ) : null}
      {tab.roundTrip === "lossy" && !alwaysLivePreview ? (
        <RoundTripNotice />
      ) : null}

      {/* The tab bar above already names the file; repeating it here was
          redundant. This row is just the mode toggle. */}
      <div className="flex shrink-0 items-center justify-end border-b border-border/60 px-4 py-1.5">
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
          headings={headings}
          key={tab.path}
          noteIndex={noteIndex}
          onActiveHeadingChange={onActiveHeadingChange}
          onChange={onChange}
          onHeadingsChange={onHeadingsChange}
          onRegisterScroll={onRegisterScroll}
          onSave={onSave}
          onWikilinkClick={onWikilinkClick}
          tab={tab}
        />
      )}
    </div>
  );
}
