import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Markdown } from "tiptap-markdown";
import { useEffect } from "react";
import { ChipNode } from "@/features/composer/chipNode";

/** Safe formatted content for the local conversation loop; raw HTML is disabled. */
export function MessageContent({ content }: { content: string }) {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        link: {
          openOnClick: true,
          protocols: ["buzz"],
          HTMLAttributes: { rel: "noopener noreferrer", target: "_blank" },
        },
      }),
      ChipNode,
      Markdown.configure({ html: false, breaks: true }),
    ],
    content,
    editable: false,
    editorProps: { attributes: { "aria-label": "Message content" } },
  });
  useEffect(() => {
    editor?.commands.setContent(content, { emitUpdate: false });
  }, [editor, content]);
  return <EditorContent editor={editor} />;
}
