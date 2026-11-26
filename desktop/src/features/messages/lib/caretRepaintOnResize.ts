import { Extension } from "@tiptap/core";
import { Plugin } from "@tiptap/pm/state";

// WebKit keeps painting the caret where it was before the bottom-anchored composer grew;
// re-adding the same range clears it (cf. ProseMirror/prosemirror#1092, which only covers selection changes).
export const CaretRepaintOnResize = Extension.create({
  name: "caretRepaintOnResize",

  addProseMirrorPlugins() {
    return [
      new Plugin({
        view(view) {
          if (typeof ResizeObserver === "undefined") return {};
          const observer = new ResizeObserver(() => {
            if (view.composing || !view.hasFocus()) return;
            const selection = view.dom.ownerDocument.getSelection();
            if (!selection?.isCollapsed || selection.rangeCount === 0) return;
            const range = selection.getRangeAt(0).cloneRange();
            if (!view.dom.contains(range.startContainer)) return;
            selection.removeAllRanges();
            selection.addRange(range);
          });
          observer.observe(view.dom);
          return { destroy: () => observer.disconnect() };
        },
      }),
    ];
  },
});
