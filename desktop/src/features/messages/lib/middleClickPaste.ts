import { TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";

import { isLinuxPlatform } from "@/shared/lib/platform";

/** A middle-button release pastes immediately; anything later is unrelated. */
const MIDDLE_CLICK_PASTE_WINDOW_MS = 1000;

/**
 * Middle-click paste of the X11/Wayland primary selection.
 *
 * WebKitGTK answers a middle click with a `paste` event whose `clipboardData`
 * holds the Ctrl+C clipboard, while its own native insertion uses the primary
 * selection. ProseMirror reads `clipboardData`, so the composer pasted the
 * clipboard instead of the selected text. Only an empty clipboard fell through
 * to ProseMirror's capture fallback, which is why it sometimes worked.
 *
 * Middle-click paste only exists on Linux. For a paste that follows a
 * middle-button press there, this ignores `clipboardData` and moves focus to
 * an off-screen textarea so WebKit's native insertion lands there, then feeds
 * that text through ProseMirror's paste pipeline at the clicked position. The `paste` handler must run before any other composer
 * paste handling, which would otherwise act on the wrong text.
 */
export function createMiddleClickPasteHandlers() {
  let pending: { at: number; left: number; top: number } | null = null;

  return {
    mousedown(_view: EditorView, event: Event): boolean {
      if (
        event instanceof MouseEvent &&
        event.button === 1 &&
        isLinuxPlatform()
      ) {
        pending = {
          at: event.timeStamp,
          left: event.clientX,
          top: event.clientY,
        };
      }
      return false;
    },

    paste(view: EditorView, event: Event): boolean {
      const click = pending;
      pending = null;
      if (
        !click ||
        !view.editable ||
        event.timeStamp - click.at > MIDDLE_CLICK_PASTE_WINDOW_MS ||
        !view.dom.parentNode
      ) {
        return false;
      }

      const capture = document.createElement("textarea");
      capture.style.cssText = "position: fixed; left: -10000px; top: 10px";
      view.dom.parentNode.appendChild(capture);
      capture.focus();

      window.setTimeout(() => {
        const text = capture.value;
        capture.remove();
        if (!view.dom.isConnected) return;
        view.focus();
        if (!text) return;
        const position = view.posAtCoords(click);
        if (position) {
          view.dispatch(
            view.state.tr.setSelection(
              TextSelection.create(view.state.doc, position.pos),
            ),
          );
        }
        const clipboardData = new DataTransfer();
        clipboardData.setData("text/plain", text);
        view.pasteText(text, new ClipboardEvent("paste", { clipboardData }));
      }, 50);
      return true;
    },
  };
}
