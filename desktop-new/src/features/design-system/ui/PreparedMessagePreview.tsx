import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Markdown } from "tiptap-markdown";

/**
 * Documentation-only round trip through the composer's existing parser.
 * This is not Rich Content or a new product formatting implementation.
 * Each immutable preview message mounts once with its submitted content.
 */
export function PreparedMessagePreview({ content }: { content: string }) {
  const editor = useEditor({
    extensions: [StarterKit, Markdown.configure({ html: false })],
    content,
    editable: false,
    editorProps: { attributes: { "aria-label": "Preview message content" } },
  });
  return <EditorContent editor={editor} />;
}
