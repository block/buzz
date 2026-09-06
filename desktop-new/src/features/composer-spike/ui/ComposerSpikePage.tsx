import { useEffect, type MouseEvent as ReactMouseEvent } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";

import { SpikeTileNode, TILE_NODE_NAME } from "../tileNode";

declare global {
  interface Window {
    /** Spike-only: lets the test read live editor state without render timing. */
    __SPIKE_READ__?: () => {
      json: unknown;
      text: string;
      addresses: string[];
    };
  }
}

/**
 * Composition-input spike harness.
 *
 * Not a product surface and not a design-system specimen. It exists so a test
 * can drive real `compositionstart` / `compositionupdate` / `compositionend`
 * sequences against an inline atom tile in the actual application shell, and
 * read back whether the document survived. Delete this route once the spike's
 * result is recorded in the plan.
 */
export function ComposerSpikePage() {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({ heading: false, link: false }),
      SpikeTileNode,
    ],
    content: { type: "doc", content: [{ type: "paragraph" }] },
    editorProps: {
      attributes: {
        "data-testid": "spike-editor",
        "aria-label": "Composition spike composer",
        class: "spike-editor",
      },
    },
  });

  // Read live editor state on demand. A React-rendered readout lags editor
  // transactions, which made the harness look like a browser defect.
  useEffect(() => {
    if (!editor) return;
    window.__SPIKE_READ__ = () => {
      const json = editor.getJSON() as {
        content?: {
          content?: { type: string; attrs?: Record<string, unknown> }[];
        }[];
      };
      const inline = json.content?.[0]?.content ?? [];
      return {
        json,
        text: editor.getText(),
        addresses: inline
          .filter((node) => node.type === TILE_NODE_NAME)
          .map((node) => `${node.attrs?.kind}/${node.attrs?.id}`),
      };
    };
    return () => {
      window.__SPIKE_READ__ = undefined;
    };
  }, [editor]);

  if (!editor) return null;

  // Insert buttons must not take focus, or the caret leaves the editor and
  // every keyboard assertion afterwards measures the wrong thing. A real
  // picker has the same obligation.
  const keepFocus = (event: ReactMouseEvent) => event.preventDefault();

  const insertTile = (label: string, id: string) =>
    editor
      .chain()
      .focus()
      .insertContent({
        type: TILE_NODE_NAME,
        attrs: { kind: "person", id, label },
      })
      .run();

  return (
    <div className="flex flex-col gap-4 p-8">
      <h1 className="text-title text-primary">Composition input spike</h1>
      <p className="text-body text-secondary">
        Drives staged character input beside an inline atom tile. Temporary.
      </p>

      <div className="flex gap-2">
        <button
          type="button"
          data-testid="insert-morgan"
          onMouseDown={keepFocus}
          onClick={() => insertTile("Morgan", "pk-morgan")}
          className="rounded-md bg-inset px-3 py-1 text-body text-primary"
        >
          Insert Morgan
        </button>
        <button
          type="button"
          data-testid="insert-alex"
          onMouseDown={keepFocus}
          onClick={() => insertTile("Alex", "pk-alex")}
          className="rounded-md bg-inset px-3 py-1 text-body text-primary"
        >
          Insert Alex
        </button>
        <button
          type="button"
          data-testid="clear"
          onMouseDown={keepFocus}
          onClick={() => editor.chain().focus().clearContent(true).run()}
          className="rounded-md bg-inset px-3 py-1 text-body text-primary"
        >
          Clear
        </button>
      </div>

      <div className="rounded-md border border-secondary bg-panel p-3">
        <EditorContent editor={editor} />
      </div>

      <output
        data-testid="doc-json"
        className="whitespace-pre-wrap font-mono text-mono-sm text-secondary"
      >
        {JSON.stringify(editor.getJSON())}
      </output>
      <output
        data-testid="doc-text"
        className="whitespace-pre-wrap font-mono text-mono-sm text-secondary"
      >
        {editor.getText()}
      </output>
      <output
        data-testid="tile-count"
        className="font-mono text-mono-sm text-secondary"
      >
        {String(
          editor
            .getJSON()
            .content?.[0]?.content?.filter(
              (n: { type: string }) => n.type === TILE_NODE_NAME,
            ).length ?? 0,
        )}
      </output>
    </div>
  );
}
