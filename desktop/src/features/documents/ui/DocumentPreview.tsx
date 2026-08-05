import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { cn } from "@/shared/lib/cn";
import { SyntaxHighlightedCode } from "@/shared/ui/markdown/CodeBlock";

/**
 * Read-only rendering of a vault note.
 *
 * Deliberately *not* `shared/ui/markdown`'s `Markdown`: that component resolves
 * relay media URLs, `@`-mentions, channel links, and agent-snapshot
 * attachments. Those are chat semantics, and applying them to a file on disk
 * would rewrite links the user never meant as Buzz references. A vault note is
 * plain CommonMark + GFM.
 *
 * Phase 2 replaces this with the live TipTap editor; it stays for the
 * round-trip-unsafe case, where showing rendered output is still useful.
 */
export function DocumentPreview({
  className,
  content,
}: {
  className?: string;
  content: string;
}) {
  return (
    <div
      className={cn(
        "prose prose-sm dark:prose-invert max-w-none break-words",
        className,
      )}
      data-testid="document-preview"
    >
      <ReactMarkdown
        components={{
          code({ children, className: codeClassName, ...props }) {
            const language = /language-(\w+)/.exec(codeClassName ?? "")?.[1];
            const code = String(children).replace(/\n$/, "");
            if (!language) {
              return (
                <code className={codeClassName} {...props}>
                  {children}
                </code>
              );
            }
            return <SyntaxHighlightedCode code={code} language={language} />;
          },
        }}
        remarkPlugins={[remarkGfm]}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
