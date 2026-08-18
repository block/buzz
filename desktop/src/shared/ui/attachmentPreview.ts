export const MAX_TEXT_PREVIEW_BYTES = 2 * 1024 * 1024;

export type AttachmentPreviewKind =
  | { kind: "markdown" }
  | { kind: "text"; language?: string }
  | { kind: "pdf" }
  | { kind: "none" };

const MARKDOWN_EXTENSIONS = new Set(["md", "markdown", "mdown", "mkd"]);
const PLAIN_TEXT_EXTENSIONS = new Set(["txt", "text", "log"]);

const CODE_LANGUAGES: Readonly<Record<string, string>> = {
  asm: "asm",
  bash: "bash",
  bat: "bat",
  c: "c",
  cc: "cpp",
  cfg: "ini",
  clj: "clojure",
  cljs: "clojure",
  cmd: "bat",
  conf: "ini",
  cpp: "cpp",
  cs: "csharp",
  css: "css",
  csv: "csv",
  dart: "dart",
  diff: "diff",
  env: "dotenv",
  erl: "erlang",
  ex: "elixir",
  exs: "elixir",
  fs: "fsharp",
  fsx: "fsharp",
  go: "go",
  gql: "graphql",
  gradle: "groovy",
  graphql: "graphql",
  groovy: "groovy",
  h: "c",
  hcl: "hcl",
  hpp: "cpp",
  html: "html",
  ini: "ini",
  java: "java",
  js: "javascript",
  json: "json",
  jsonc: "jsonc",
  jsonl: "jsonl",
  jsx: "jsx",
  kt: "kotlin",
  kts: "kotlin",
  lua: "lua",
  mdx: "mdx",
  patch: "diff",
  pl: "perl",
  pm: "perl",
  php: "php",
  properties: "properties",
  proto: "proto",
  ps1: "powershell",
  py: "python",
  r: "r",
  rb: "ruby",
  rs: "rust",
  scala: "scala",
  scss: "scss",
  sh: "bash",
  sol: "solidity",
  sql: "sql",
  svelte: "svelte",
  swift: "swift",
  tf: "hcl",
  toml: "toml",
  ts: "typescript",
  tsv: "tsv",
  tsx: "tsx",
  vue: "vue",
  xml: "xml",
  yaml: "yaml",
  yml: "yaml",
  zig: "zig",
  zsh: "bash",
};

const SPECIAL_FILENAMES: Readonly<Record<string, string>> = {
  ".editorconfig": "ini",
  ".env": "dotenv",
  ".gitattributes": "git-commit",
  ".gitignore": "git-commit",
  ".npmrc": "ini",
  dockerfile: "dockerfile",
  gemfile: "ruby",
  makefile: "makefile",
  procfile: "shellscript",
};

function cleanFilename(filename: string): string {
  const withoutQuery = filename.split(/[?#]/, 1)[0] ?? filename;
  return withoutQuery.split(/[\\/]/).pop()?.trim().toLowerCase() ?? "";
}

export function classifyAttachmentPreview(
  filename: string,
  mimeType?: string,
  sourceUrl?: string,
): AttachmentPreviewKind {
  const lowerName = cleanFilename(filename);
  const extension = lowerName.includes(".")
    ? (lowerName.split(".").pop() ?? "")
    : "";

  if (MARKDOWN_EXTENSIONS.has(extension)) return { kind: "markdown" };
  if (extension === "pdf") return { kind: "pdf" };
  if (PLAIN_TEXT_EXTENSIONS.has(extension)) return { kind: "text" };

  const language = CODE_LANGUAGES[extension] ?? SPECIAL_FILENAMES[lowerName];
  if (language) return { kind: "text", language };

  // Agent-authored Markdown links sometimes use a friendly label rather than
  // the real filename. When that label has no extension, the canonical relay
  // media URL still carries the validated upload extension.
  if (!extension && sourceUrl) {
    const sourceKind = classifyAttachmentPreview(sourceUrl, mimeType);
    if (sourceKind.kind !== "none") return sourceKind;
  }

  // A PDF may have no useful filename (for example an older relay link), but
  // MIME alone must not turn an arbitrary .bin payload into executable text.
  if (mimeType?.split(";", 1)[0].trim().toLowerCase() === "application/pdf") {
    return { kind: "pdf" };
  }

  return { kind: "none" };
}

export function decodeTextPreview(bytes: Uint8Array): string {
  if (bytes.byteLength > MAX_TEXT_PREVIEW_BYTES) {
    throw new Error("This file is too large to preview (2 MB maximum).");
  }

  // NULs are a strong signal that a file with a text-looking extension is
  // actually binary. Reject it instead of filling the dialog with garbage.
  if (bytes.includes(0)) {
    throw new Error("This file does not contain supported UTF-8 text.");
  }

  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error("This file does not contain supported UTF-8 text.");
  }
}
