import { type MouseEvent as ReactMouseEvent, useEffect } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";

import { TILE_NODE_NAME, TileNode } from "@/features/composer/tileNode";
import type { TileAddress } from "@/shared/tiles/address";
import { resetTileFaces, tileFaces } from "@/shared/tiles/faceResolver";

declare global {
  interface Window {
    /** Spike-only: lets a test read live editor state without render timing. */
    __SPIKE_READ__?: () => {
      json: unknown;
      text: string;
      addresses: string[];
    };
    /** Spike-only: renames an identity the way a profile update would. */
    __SPIKE_RENAME__?: (address: TileAddress, label: string) => void;
    /** Spike-only: simulates switching community. */
    __SPIKE_RESET_FACES__?: () => void;
  }
}

const MORGAN: TileAddress = { kind: "person", id: "pk-morgan" };
const ALEX: TileAddress = { kind: "person", id: "pk-alex" };

/**
 * Harness for the tile composer's editing contract.
 *
 * Not a product surface. It mounts the real TileNode, the real node view, and
 * the real shared InlineTile so browser tests bind production seams rather
 * than a test-only stand-in. Delete it once the product composer exists,
 * folding its assertions into that composer's tests.
 */
export function ComposerSpikePage() {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({ heading: false, link: false }),
      TileNode,
    ],
    content: { type: "doc", content: [{ type: "paragraph" }] },
    editorProps: {
      attributes: {
        "data-testid": "spike-editor",
        "aria-label": "Tile composer harness",
        class: "spike-editor",
      },
    },
  });

  // Seed the faces these tiles resolve to. The product resolves them from real
  // identity; the harness only needs them present. Deliberately no cleanup:
  // resetting on unmount would make a community-reset test tear down the very
  // harness it is measuring.
  useEffect(() => {
    tileFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });
    tileFaces.put(ALEX, { label: "Alex", loading: false, resolved: true });
  }, []);

  // Read live editor state on demand. A React-rendered readout lags editor
  // transactions, which made an earlier harness look like a browser defect.
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
    window.__SPIKE_RENAME__ = (address, label) =>
      tileFaces.put(address, { label, loading: false, resolved: true });
    window.__SPIKE_RESET_FACES__ = () => resetTileFaces();
    return () => {
      window.__SPIKE_READ__ = undefined;
      window.__SPIKE_RENAME__ = undefined;
      window.__SPIKE_RESET_FACES__ = undefined;
    };
  }, [editor]);

  if (!editor) return null;

  // Insert controls must not take focus, or the caret leaves the editor and
  // every keyboard assertion afterwards measures the wrong thing. The real
  // tile picker owes the same.
  const keepFocus = (event: ReactMouseEvent) => event.preventDefault();

  const insertTile = (address: TileAddress) =>
    editor
      .chain()
      .focus()
      .insertContent({ type: TILE_NODE_NAME, attrs: address })
      .run();

  return (
    <div className="flex flex-col gap-4 p-8">
      <h1 className="text-title text-primary">Tile composer harness</h1>
      <p className="text-body text-secondary">
        Exercises the production tile node, node view, and shared InlineTile.
        Temporary.
      </p>

      <div className="flex gap-2">
        <button
          type="button"
          data-testid="insert-morgan"
          onMouseDown={keepFocus}
          onClick={() => insertTile(MORGAN)}
          className="rounded-md bg-inset px-3 py-1 text-body text-primary"
        >
          Insert Morgan
        </button>
        <button
          type="button"
          data-testid="insert-alex"
          onMouseDown={keepFocus}
          onClick={() => insertTile(ALEX)}
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
    </div>
  );
}
