import { StickyNote, Trash2 } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  bestieScratchSnippet,
  deriveBestieScratchTitle,
} from "./bestieScratchStorage";
import {
  addBestieScratchNoteForScope,
  removeBestieScratchNoteForScope,
  updateBestieScratchNoteForScope,
  useBestieScratch,
} from "./bestieScratchStore";
import type {
  BestieScratchNote,
  BestieScratchScope,
} from "./bestieScratchTypes";

function ScratchListRow({
  note,
  onOpen,
  onRemove,
}: {
  note: BestieScratchNote;
  onOpen: () => void;
  onRemove: () => void;
}) {
  const snippet = bestieScratchSnippet(note.body);
  return (
    <div
      className="group flex items-start gap-2 rounded-md border border-border/60 bg-muted/25 px-2 py-1.5"
      data-testid={`bestie-scratch-item-${note.id}`}
    >
      <button
        className="min-w-0 flex-1 text-left"
        data-testid={`bestie-scratch-open-${note.id}`}
        onClick={onOpen}
        type="button"
      >
        <p className="flex items-center gap-1.5 text-sm font-medium leading-snug">
          <StickyNote className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate">{note.title}</span>
        </p>
        {snippet && snippet !== note.title ? (
          <p className="mt-0.5 line-clamp-2 text-2xs text-muted-foreground">
            {snippet}
          </p>
        ) : null}
      </button>
      <Button
        aria-label="Delete scratch note"
        className="mt-0.5 size-6 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
        data-testid={`bestie-scratch-remove-${note.id}`}
        onClick={onRemove}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        <Trash2 className="size-3.5" />
      </Button>
    </div>
  );
}

const SCRATCH_BODY_MIN_PX = 112; // ~min-h-28
const SCRATCH_BODY_DEFAULT_PX = 160;
const SCRATCH_BODY_MAX_PX = 480; // stay inside the Lists sheet

function ScratchEditor({
  note,
  onClose,
  onSave,
}: {
  note: BestieScratchNote;
  onClose: () => void;
  onSave: (input: { body: string; title: string }) => void;
}) {
  const [title, setTitle] = React.useState(note.title);
  const [body, setBody] = React.useState(note.body);
  const [bodyHeight, setBodyHeight] = React.useState(SCRATCH_BODY_DEFAULT_PX);
  const titleRef = React.useRef<HTMLInputElement>(null);
  const dragRef = React.useRef<{ startY: number; startH: number } | null>(null);
  React.useEffect(() => {
    titleRef.current?.focus();
    setBodyHeight(SCRATCH_BODY_DEFAULT_PX);
  }, [note.id]);

  const save = () => {
    const nextTitle = title.trim() || deriveBestieScratchTitle(body);
    onSave({ body, title: nextTitle });
  };

  const onResizePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    dragRef.current = { startY: event.clientY, startH: bodyHeight };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onResizePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    const next = Math.min(
      SCRATCH_BODY_MAX_PX,
      Math.max(SCRATCH_BODY_MIN_PX, drag.startH + (event.clientY - drag.startY)),
    );
    setBodyHeight(next);
  };
  const onResizePointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  return (
    <div
      className="flex flex-col gap-1.5 rounded-md border border-border/60 bg-muted/25 p-2"
      data-testid={`bestie-scratch-editor-${note.id}`}
    >
      <Input
        aria-label="Scratch title"
        className="h-8 text-sm"
        data-testid="bestie-scratch-title"
        onChange={(event) => setTitle(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
        placeholder="Title"
        ref={titleRef}
        value={title}
      />
      <Textarea
        aria-label="Scratch body"
        className="resize-none overflow-y-auto text-sm"
        data-testid="bestie-scratch-body"
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
        placeholder="Notes…"
        style={{ height: bodyHeight }}
        value={body}
      />
      <div
        aria-label="Drag to resize note"
        className="flex h-3 cursor-ns-resize items-center justify-center rounded-sm hover:bg-muted/60"
        data-testid="bestie-scratch-resize"
        onPointerDown={onResizePointerDown}
        onPointerMove={onResizePointerMove}
        onPointerUp={onResizePointerUp}
        onPointerCancel={onResizePointerUp}
        role="separator"
        title="Drag to make taller"
      >
        <span className="h-0.5 w-8 rounded-full bg-border" />
      </div>
      <div className="flex items-center justify-between gap-1.5">
        <Button
          className="h-7 px-2 text-xs"
          data-testid="bestie-scratch-expand"
          onClick={() =>
            setBodyHeight((h) =>
              Math.min(SCRATCH_BODY_MAX_PX, h + 96),
            )
          }
          size="sm"
          type="button"
          variant="ghost"
        >
          Read more
        </Button>
        <div className="flex gap-1.5">
          <Button
            className="h-7 px-2 text-xs"
            data-testid="bestie-scratch-cancel"
            onClick={onClose}
            size="sm"
            type="button"
            variant="ghost"
          >
            Close
          </Button>
          <Button
            className="h-7 px-2 text-xs"
            data-testid="bestie-scratch-save"
            onClick={save}
            size="sm"
            type="button"
            variant="secondary"
          >
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}

function AddScratchRow({
  onAdd,
}: {
  onAdd: (input: { body: string; title: string }) => void;
}) {
  const [title, setTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const titleRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    titleRef.current?.focus();
  }, []);

  const submit = () => {
    const nextTitle = title.trim();
    const nextBody = body.trimEnd();
    if (!nextTitle && !nextBody.trim()) return;
    onAdd({
      body: nextBody,
      title: nextTitle || deriveBestieScratchTitle(nextBody),
    });
    setTitle("");
    setBody("");
  };

  return (
    <div className="flex flex-col gap-1.5" data-testid="bestie-add-scratch">
      <Input
        aria-label="Scratch title"
        className="h-8 text-sm"
        onChange={(event) => setTitle(event.target.value)}
        placeholder="Title"
        ref={titleRef}
        value={title}
      />
      <Textarea
        aria-label="Scratch body"
        className="min-h-20 resize-none text-sm"
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            submit();
          }
        }}
        placeholder="Park a thought…"
        value={body}
      />
      <div className="flex justify-end">
        <Button
          className="h-7 px-2 text-xs"
          data-testid="bestie-add-scratch-submit"
          disabled={!title.trim() && !body.trim()}
          onClick={submit}
          size="sm"
          type="button"
          variant="secondary"
        >
          Add
        </Button>
      </div>
    </div>
  );
}

/**
 * Scratch category sheet — lightweight personal pad (title/snippet list,
 * click to edit body). Not a wiki.
 */
export function BestieDmScratchSheet({
  adding,
  onAdded,
  onRequestAdd,
  scope,
}: {
  adding: boolean;
  /** Close the add form after a successful create — user must click Add again. */
  onAdded?: () => void;
  onRequestAdd?: () => void;
  scope: BestieScratchScope;
}) {
  const state = useBestieScratch(scope);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const editing = state.notes.find((note) => note.id === editingId) ?? null;

  return (
    <div
      className="flex flex-col gap-3 py-1"
      data-testid="bestie-dm-scratch-sheet"
    >
      {editing ? (
        <ScratchEditor
          note={editing}
          onClose={() => setEditingId(null)}
          onSave={(input) => {
            updateBestieScratchNoteForScope(scope, {
              body: input.body,
              id: editing.id,
              title: input.title,
            });
            setEditingId(null);
          }}
        />
      ) : (
        <>
          {adding ? (
            <AddScratchRow
              onAdd={(input) => {
                addBestieScratchNoteForScope(scope, input);
                onAdded?.();
              }}
            />
          ) : (
            <p className="px-0.5 text-xs text-muted-foreground">
              Lightweight personal pad — park a thought, not a wiki. Use + to add,
              or ask Assistant to{" "}
              <code className="text-2xs">park this: …</code>
              {onRequestAdd ? (
                <>
                  {" "}
                  <button
                    className="underline-offset-2 hover:underline"
                    onClick={onRequestAdd}
                    type="button"
                  >
                    Add one
                  </button>
                </>
              ) : null}
            </p>
          )}
          <div className="space-y-1.5">
            {state.notes.length === 0 ? (
              <p className="px-0.5 text-xs text-muted-foreground">
                No scratch notes yet.
              </p>
            ) : (
              state.notes.map((note) => (
                <ScratchListRow
                  key={note.id}
                  note={note}
                  onOpen={() => setEditingId(note.id)}
                  onRemove={() => {
                    removeBestieScratchNoteForScope(scope, note.id);
                  }}
                />
              ))
            )}
          </div>
        </>
      )}
    </div>
  );
}
