import { Extension } from "@tiptap/core";
import type { ResolvedPos } from "@tiptap/pm/model";
import { Plugin, TextSelection } from "@tiptap/pm/state";

import { isMacPlatform } from "@/shared/lib/platform";

function hasBlankLineBefore($from: ResolvedPos): boolean {
  const parentStart = $from.start();
  let previousNodeName: string | undefined;
  let blankLineBefore = false;

  $from.parent.forEach((node, offset) => {
    if (parentStart + offset >= $from.pos) return;
    if (node.type.name === "hardBreak") {
      blankLineBefore = previousNodeName === "hardBreak";
    }
    previousNodeName = node.type.name;
  });

  return blankLineBefore;
}

export const PreserveMacReplacementSelection = Extension.create({
  name: "preserveMacReplacementSelection",

  addProseMirrorPlugins() {
    let pendingSelection: { docSize: number; position: number } | null = null;

    return [
      new Plugin({
        props: {
          handleDOMEvents: {
            beforeinput(view, event) {
              const inputEvent = event as InputEvent;
              const { selection } = view.state;
              if (
                !isMacPlatform() ||
                inputEvent.inputType !== "insertReplacementText" ||
                inputEvent.data !== null ||
                !selection.empty ||
                !hasBlankLineBefore(selection.$from)
              ) {
                pendingSelection = null;
                return false;
              }

              pendingSelection = {
                docSize: view.state.doc.content.size,
                position: selection.from,
              };
              return false;
            },
            input(view, event) {
              const inputEvent = event as InputEvent;
              const capturedSelection = pendingSelection;
              pendingSelection = null;
              if (
                inputEvent.inputType !== "insertReplacementText" ||
                !capturedSelection
              ) {
                return false;
              }

              window.setTimeout(() => {
                if (!view.dom.isConnected) return;

                const { state } = view;
                const { position, docSize } = capturedSelection;
                if (
                  !state.selection.empty ||
                  state.doc.content.size !== docSize ||
                  state.selection.from >= position ||
                  position > state.doc.content.size
                ) {
                  return;
                }

                const $position = state.doc.resolve(position);
                if (!$position.parent.inlineContent) return;

                view.dispatch(
                  state.tr.setSelection(
                    TextSelection.create(state.doc, position),
                  ),
                );
              }, 0);

              return false;
            },
          },
        },
      }),
    ];
  },
});
