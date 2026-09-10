import { type MouseEvent as ReactMouseEvent, useEffect } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";

import { CHIP_NODE_NAME, ChipNode } from "@/features/composer/chipNode";
import type { ChipAddress } from "@/shared/chips/address";
import { resetChipFaces, chipFaces } from "@/shared/chips/faceResolver";

declare global {
  interface Window {
    /** Spike-only: lets a test read live editor state without render timing. */
    __SPIKE_READ__?: () => {
      json: unknown;
      text: string;
      addresses: string[];
    };
    /** Spike-only: renames an identity the way a profile update would. */
    __SPIKE_RENAME__?: (address: ChipAddress, label: string) => void;
    /** Spike-only: simulates switching community. */
    __SPIKE_RESET_FACES__?: () => void;
  }
}

const MORGAN: ChipAddress = { kind: "person", id: "pk-morgan" };
const ALEX: ChipAddress = { kind: "person", id: "pk-alex" };

/**
 * Harness for the chip composer's editing contract.
 *
 * Not a product surface. It mounts the real ChipNode, the real node view, and
 * the real shared InlineChip so browser tests bind production seams rather
 * than a test-only stand-in. Delete it once the product composer exists,
 * folding its assertions into that composer's tests.
 */
export function ComposerSpikePage() {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({ heading: false, link: false }),
      ChipNode,
    ],
    content: { type: "doc", content: [{ type: "paragraph" }] },
    editorProps: {
      attributes: {
        "data-testid": "spike-editor",
        "aria-label": "Chip composer harness",
        class: "spike-editor",
      },
    },
  });

  // Seed the faces these chips resolve to. The product resolves them from real
  // identity; the harness only needs them present. Deliberately no cleanup:
  // resetting on unmount would make a community-reset test tear down the very
  // harness it is measuring.
  useEffect(() => {
    chipFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });
    chipFaces.put(ALEX, { label: "Alex", loading: false, resolved: true });
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
          .filter((node) => node.type === CHIP_NODE_NAME)
          .map((node) => `${node.attrs?.kind}/${node.attrs?.id}`),
      };
    };
    window.__SPIKE_RENAME__ = (address, label) =>
      chipFaces.put(address, { label, loading: false, resolved: true });
    window.__SPIKE_RESET_FACES__ = () => resetChipFaces();
    return () => {
      window.__SPIKE_READ__ = undefined;
      window.__SPIKE_RENAME__ = undefined;
      window.__SPIKE_RESET_FACES__ = undefined;
    };
  }, [editor]);

  if (!editor) return null;

  // Insert controls must not take focus, or the caret leaves the editor and
  // every keyboard assertion afterwards measures the wrong thing. The real
  // chip picker owes the same.
  const keepFocus = (event: ReactMouseEvent) => event.preventDefault();

  const insertChip = (address: ChipAddress) =>
    editor
      .chain()
      .focus()
      .insertContent({ type: CHIP_NODE_NAME, attrs: address })
      .run();

  return (
    <div className="flex flex-col gap-4 p-8">
      <h1 className="text-title text-primary">Chip composer harness</h1>
      <p className="text-body text-secondary">
        Exercises the production chip node, node view, and shared InlineChip.
        Temporary.
      </p>

      <div className="flex gap-2">
        <button
          type="button"
          data-testid="insert-morgan"
          onMouseDown={keepFocus}
          onClick={() => insertChip(MORGAN)}
          className="rounded-md bg-neutral-2 px-3 py-1 text-body text-primary"
        >
          Insert Morgan
        </button>
        <button
          type="button"
          data-testid="insert-alex"
          onMouseDown={keepFocus}
          onClick={() => insertChip(ALEX)}
          className="rounded-md bg-neutral-2 px-3 py-1 text-body text-primary"
        >
          Insert Alex
        </button>
        <button
          type="button"
          data-testid="clear"
          onMouseDown={keepFocus}
          onClick={() => editor.chain().focus().clearContent(true).run()}
          className="rounded-md bg-neutral-2 px-3 py-1 text-body text-primary"
        >
          Clear
        </button>
      </div>

      <div className="rounded-md border border-primary bg-panel p-3">
        <EditorContent editor={editor} />
      </div>
    </div>
  );
}
