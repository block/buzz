import { Extension } from "@tiptap/core";
import {
  Plugin,
  PluginKey,
  type EditorState,
  type Transaction,
} from "@tiptap/pm/state";

/**
 * Ownership of the text one paste inserted, tracked across later edits.
 *
 * A pasted mention's identity can only be bound once trusted state vouches for
 * it, and that check can need a relay round trip — so the answer lands well
 * after the insertion. "Is this label visible in the composer?" is not a fence
 * for that: the label the user deleted and then hand-typed reads the same as
 * the one this paste put there, and binding the former hands a stranger's
 * pubkey to text the composer's own candidates should have resolved.
 *
 * The plugin holds a range per in-flight claim and maps it through every
 * transaction, so settlement can ask the narrower question: does the label
 * still occupy the text *this* paste owns? A range dies when its content is
 * deleted or replaced, which is the fail-closed direction — a settlement with
 * no live range binds nothing.
 *
 * The ranges callers track are the individual `@Label` tokens rather than the
 * whole insertion, so an edit elsewhere in a pasted sentence costs nothing
 * while any edit to the token itself revokes the identity it carried. The
 * plugin stays granularity-agnostic: it tracks whatever range it is handed.
 */
export const pastedMentionOccurrencesKey = new PluginKey<PastedMentionRanges>(
  "pastedMentionOccurrences",
);

/** A half-open document range, in the coordinates the editor state uses. */
export type PastedMentionRange = { from: number; to: number };

type PastedMentionRanges = ReadonlyMap<number, PastedMentionRange>;

type PastedMentionOccurrenceCommand =
  | { type: "track"; id: number; from: number; to: number }
  | { type: "release"; id: number };

/**
 * The slice of `EditorView` these helpers use.
 *
 * `EditorView` satisfies it structurally, so production passes the real view
 * while a test can drive the real plugin over an `EditorState` without a DOM.
 */
export type PastedMentionOccurrenceView = {
  state: EditorState;
  dispatch: (tr: Transaction) => void;
  isDestroyed?: boolean;
};

/**
 * Every paste releases its ranges at settlement, so this cap is a backstop
 * rather than a working limit: a composer whose settlements somehow stop
 * arriving must not accumulate ranges for the life of the session. Callers
 * also read it as the ceiling on what one paste may track — trimming costs a
 * pasted identity rather than binding one, which is the safe direction.
 */
export const MAX_TRACKED_OCCURRENCES = 50;

let nextOccurrenceId = 1;

/**
 * Map one tracked range through a transaction, or `null` once it is dead.
 *
 * Two rules, and endpoint mapping alone is neither of them. `from` maps with
 * assoc 1 and `to` with assoc −1, so text typed at either edge lands *outside*
 * the range: an occurrence owns what the paste inserted and nothing the user
 * added around it. But both endpoints also survive an edit wholly *inside* the
 * range, so endpoint mapping on its own left a range alive while the user
 * replaced the very characters it was tracking.
 *
 * Each step is therefore checked for a replaced region that strictly overlaps
 * the range first, and any overlap kills it — the range no longer owns what it
 * was handed. Touching a boundary is not an overlap, since that text was
 * always outside; a pure insertion replaces nothing at all, and the caller's
 * own check on the surviving text is what refuses those.
 */
function remapPastedMentionRange(
  range: PastedMentionRange,
  tr: Transaction,
): PastedMentionRange | null {
  let { from, to } = range;
  for (const map of tr.mapping.maps) {
    let overlapped = false;
    map.forEach((oldStart, oldEnd) => {
      if (oldEnd > oldStart && oldStart < to && oldEnd > from)
        overlapped = true;
    });
    if (overlapped) return null;
    const mappedFrom = map.mapResult(from, 1);
    const mappedTo = map.mapResult(to, -1);
    if (mappedFrom.deleted || mappedTo.deleted) return null;
    from = mappedFrom.pos;
    to = mappedTo.pos;
    if (to <= from) return null;
  }
  return { from, to };
}

function remapPastedMentionRanges(
  current: PastedMentionRanges,
  tr: Transaction,
): PastedMentionRanges {
  const next = new Map<number, PastedMentionRange>();
  for (const [id, range] of current) {
    const mapped = remapPastedMentionRange(range, tr);
    if (mapped) next.set(id, mapped);
  }
  return next;
}

function applyPastedMentionCommand(
  current: PastedMentionRanges,
  command: PastedMentionOccurrenceCommand,
): PastedMentionRanges {
  const next = new Map(current);
  if (command.type === "release") {
    next.delete(command.id);
    return next;
  }
  next.set(command.id, { from: command.from, to: command.to });
  for (const id of next.keys()) {
    if (next.size <= MAX_TRACKED_OCCURRENCES) break;
    next.delete(id);
  }
  return next;
}

/**
 * Tracks the document range each in-flight pasted mention token occupies.
 *
 * Registered in the shared composer extension list, so every composer that
 * accepts a mention paste — channel, DM, thread, edit, forum — can fence a
 * late identity binding to the text its paste still owns.
 */
export const PastedMentionOccurrencesExtension = Extension.create({
  name: "pastedMentionOccurrences",

  addProseMirrorPlugins() {
    return [
      new Plugin<PastedMentionRanges>({
        key: pastedMentionOccurrencesKey,
        state: {
          init: () => new Map(),
          apply(tr, current) {
            const mapped = tr.docChanged
              ? remapPastedMentionRanges(current, tr)
              : current;
            const command = tr.getMeta(pastedMentionOccurrencesKey) as
              | PastedMentionOccurrenceCommand
              | undefined;
            return command
              ? applyPastedMentionCommand(mapped, command)
              : mapped;
          },
        },
      }),
    ];
  },
});

/**
 * Start tracking `[from, to)` as one mention token a paste inserted.
 *
 * Returns `null` when there is nothing to own (an empty insertion) or when the
 * composer carries no occurrence plugin — callers treat that as "no live
 * occurrence" and bind nothing, so an unregistered extension costs a pasted
 * identity rather than binding one no fence can retire.
 */
export function trackPastedMentionOccurrence(
  view: PastedMentionOccurrenceView,
  from: number,
  to: number,
): number | null {
  if (view.isDestroyed || to <= from) return null;
  if (!pastedMentionOccurrencesKey.getState(view.state)) return null;
  const id = nextOccurrenceId++;
  view.dispatch(
    view.state.tr.setMeta(pastedMentionOccurrencesKey, {
      type: "track",
      id,
      from,
      to,
    }),
  );
  return id;
}

/** Where an occurrence now sits, or `null` once its range is gone. */
export function readPastedMentionOccurrenceRange(
  view: PastedMentionOccurrenceView,
  id: number | null,
): PastedMentionRange | null {
  if (id === null || view.isDestroyed) return null;
  const range = pastedMentionOccurrencesKey.getState(view.state)?.get(id);
  if (!range) return null;
  return range.to > view.state.doc.content.size ? null : range;
}

/** Stop tracking an occurrence whose paste has finished settling. */
export function releasePastedMentionOccurrence(
  view: PastedMentionOccurrenceView,
  id: number | null,
): void {
  if (id === null || view.isDestroyed) return;
  if (!pastedMentionOccurrencesKey.getState(view.state)?.has(id)) return;
  view.dispatch(
    view.state.tr.setMeta(pastedMentionOccurrencesKey, { type: "release", id }),
  );
}
