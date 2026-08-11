import * as React from "react";

import { Markdown } from "@/shared/ui/markdown";
import { SyntaxHighlightedCode } from "@/shared/ui/markdown/CodeBlock";

/** Filename extension → Shiki bundled-language id, for extensions where they differ. */
const EXTENSION_LANGUAGE_OVERRIDES: Record<string, string> = {
  cjs: "javascript",
  cfg: "ini",
  conf: "ini",
  cc: "cpp",
  hpp: "cpp",
  h: "c",
  kt: "kotlin",
  mjs: "javascript",
  ps1: "powershell",
  rs: "rust",
  sh: "bash",
  yml: "yaml",
  zsh: "bash",
};

function languageForFilename(filename: string): string {
  const lower = filename.toLowerCase();
  if (lower === "dockerfile") return "docker";
  const ext = lower.includes(".") ? (lower.split(".").pop() ?? "") : "";
  return EXTENSION_LANGUAGE_OVERRIDES[ext] ?? ext ?? "text";
}

/**
 * Decodes and displays a text/code/markdown file's bytes.
 *
 * `.md`/`.markdown` files render through the same `Markdown` component used
 * for message bodies (headings, lists, links, etc. render properly instead of
 * showing raw source). Everything else renders as syntax-highlighted code via
 * the same Shiki-backed highlighter message code blocks use.
 *
 * `interactive={false}` on the markdown path disables link-preview and
 * config-nudge parsing — an uploaded file's content isn't an authored
 * message, so entity/channel mention linking is intentionally left off.
 */
export function TextFilePreview({
  bytes,
  filename,
  mode,
}: {
  bytes: Uint8Array;
  filename: string;
  mode: "markdown" | "code";
}) {
  const text = React.useMemo(() => new TextDecoder().decode(bytes), [bytes]);

  if (mode === "markdown") {
    return (
      <div className="p-4">
        <Markdown content={text} interactive={false} />
      </div>
    );
  }

  const language = languageForFilename(filename);
  return (
    <pre className="h-full overflow-auto p-4">
      <SyntaxHighlightedCode code={text} language={language} />
    </pre>
  );
}
