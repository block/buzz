import { type Editor, isAndroid, isiOS, isSafari } from "@tiptap/core";
import { Selection } from "@tiptap/pm/state";

/** One cancellable automatic-focus request. Explicit editor commands stay untouched. */
export function scheduleComposerAutofocus(
  editor: Editor,
  disabled: () => boolean,
): () => void {
  if (editor.isDestroyed || disabled()) return () => {};
  const view = editor.view;
  const doc = view.dom.ownerDocument;
  const win = doc.defaultView;
  if (!win) return () => {};
  const active = doc.activeElement as HTMLElement | null;
  if (
    active &&
    active !== doc.body &&
    (active.matches("input, textarea, select") ||
      active.isContentEditable ||
      active.closest('[role="menu"], [role="dialog"]'))
  ) {
    return () => {};
  }

  let retired = false;
  let claimed = false;
  const claim = () => {
    claimed = true;
  };
  const focusClaim = (event: Event) => {
    if (!view.dom.contains(event.target as Node)) claim();
  };
  // Only observe while this request is pending; never suppress user events.
  doc.addEventListener("pointerdown", claim, true);
  doc.addEventListener("keydown", claim, true);
  doc.addEventListener("focusin", focusClaim, true);
  const removeListeners = () => {
    doc.removeEventListener("pointerdown", claim, true);
    doc.removeEventListener("keydown", claim, true);
    doc.removeEventListener("focusin", focusClaim, true);
  };
  const valid = () =>
    !retired &&
    !claimed &&
    !disabled() &&
    !editor.isDestroyed &&
    view.dom.isConnected &&
    editor.view === view;

  // Match Tiptap's immediate mobile/Safari preparation. Its focus command
  // cannot be used here: it queues an unconditional, uncancellable inner RAF.
  if (valid()) {
    if (isiOS() || isAndroid()) view.dom.focus();
    else if (isSafari()) view.dom.focus({ preventScroll: true });
  }
  const frame = win.requestAnimationFrame(function commitComposerAutofocus() {
    if (valid()) {
      const selection = Selection.atEnd(editor.state.doc);
      if (!editor.state.selection.eq(selection)) {
        view.dispatch(editor.state.tr.setSelection(selection));
      }
      // Selection observers can synchronously transfer focus or retire scope.
      if (valid()) {
        view.focus();
        editor.commands.scrollIntoView();
      }
    }
    removeListeners();
  });
  return () => {
    retired = true;
    win.cancelAnimationFrame(frame);
    removeListeners();
  };
}
