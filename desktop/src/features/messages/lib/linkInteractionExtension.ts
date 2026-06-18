import { Extension } from "@tiptap/core";
import { Plugin, PluginKey, TextSelection } from "@tiptap/pm/state";
import type { EditorState } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";

import { resolveLinkAt, type LinkSelectionInfo } from "./resolveLinkAt";

type LinkInteractionExtensionOptions = {
  getEditLinkHandler: () => ((info: LinkSelectionInfo) => void) | undefined;
  getSelectionChangeHandler: () =>
    | ((info: LinkSelectionInfo | null) => void)
    | undefined;
};

export const linkInteractionKey = new PluginKey("linkInteraction");

/**
 * Centralises composer link interactions that depend on ProseMirror editor
 * state: click interception, click-vs-drag selection preservation, and active
 * link reporting as the selection moves.
 */
export function createLinkInteractionExtension({
  getEditLinkHandler,
  getSelectionChangeHandler,
}: LinkInteractionExtensionOptions) {
  return Extension.create({
    name: "linkInteraction",

    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: linkInteractionKey,
          view(view) {
            notifyLinkSelection(view.state, getSelectionChangeHandler);

            return {
              update(updatedView, previousState) {
                const nextState = updatedView.state;
                if (
                  previousState.selection.eq(nextState.selection) &&
                  previousState.doc.eq(nextState.doc)
                ) {
                  return;
                }
                notifyLinkSelection(nextState, getSelectionChangeHandler);
              },
            };
          },
          props: {
            handleDOMEvents: {
              // Native anchor default can still win in the WebView before
              // ProseMirror's semantic click hook runs, so intercept editor
              // links at the DOM event layer and route them to composer-local
              // controls.
              click(view, event) {
                if (!(event instanceof MouseEvent)) return false;
                const target = event.target;
                if (!(target instanceof Element)) return false;
                const anchor = target.closest("a[href]");
                if (!anchor || !view.dom.contains(anchor)) return false;

                const position = view.posAtCoords({
                  left: event.clientX,
                  top: event.clientY,
                });
                if (!position) {
                  event.preventDefault();
                  event.stopPropagation();
                  return true;
                }
                return handleLinkClick({
                  event,
                  getEditLinkHandler,
                  pos: position.pos,
                  view,
                });
              },
            },
            // Click on an existing link -> surface composer-local link controls.
            // The link extension is configured `openOnClick: false` (never
            // navigate away from a chat composer), so without this hook a click
            // on a link does nothing.
            handleClick(view, pos, event) {
              return handleLinkClick({
                event,
                getEditLinkHandler,
                pos,
                view,
              });
            },
          },
        }),
      ];
    },
  });
}

function notifyLinkSelection(
  state: EditorState,
  getSelectionChangeHandler: LinkInteractionExtensionOptions["getSelectionChangeHandler"],
) {
  const handler = getSelectionChangeHandler();
  if (!handler) return;
  if (!state.selection.empty) {
    handler(null);
    return;
  }
  handler(resolveLinkAt(state, state.selection.from));
}

function handleLinkClick({
  event,
  getEditLinkHandler,
  pos,
  view,
}: {
  event: MouseEvent;
  getEditLinkHandler: LinkInteractionExtensionOptions["getEditLinkHandler"];
  pos: number;
  view: EditorView;
}): boolean {
  const handler = getEditLinkHandler();
  if (!handler) return false;

  const info = resolveLinkAt(view.state, pos);
  if (!info) return false;

  event.preventDefault();
  event.stopPropagation();
  if (!view.state.selection.empty) {
    return true;
  }

  view.dispatch(
    view.state.tr
      .setSelection(TextSelection.create(view.state.doc, pos))
      .scrollIntoView(),
  );
  view.focus();
  handler(info);
  return true;
}
