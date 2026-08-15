import { bundledLanguagesInfo } from "shiki";

/**
 * Decide how a message attachment should render in the file viewer panel.
 *
 * Keyed on the filename extension, not the imeta MIME: text and source files
 * have no magic bytes, so uploads routinely arrive as
 * `application/octet-stream` (see `validate_file_content` in buzz-media). A
 * MIME fallback covers extension-less names.
 *
 * Languages resolve through Shiki's own grammar ids and aliases instead of a
 * hand-maintained table; only extensions Shiki does not alias are listed here.
 */

export type FileViewKind =
  | { kind: "markdown" }
  | { kind: "code"; language: string }
  | { kind: "text" }
  | { kind: "none" };

const MARKDOWN_EXTENSIONS: Record<string, true> = {
  markdown: true,
  md: true,
  mdx: true,
};

/**
 * Extensions Shiki neither uses as a grammar id nor lists as an alias, mapped
 * to the grammar that should highlight them.
 */
const EXTENSION_LANGUAGE_OVERRIDES: Record<string, string> = {
  cjs: "javascript",
  h: "c",
  hpp: "cpp",
  mjs: "javascript",
  patch: "diff",
};

/** Extensions rendered as plain text — no grammar, no highlighting. */
const TEXT_EXTENSIONS: Record<string, true> = {
  cfg: true,
  conf: true,
  csv: true,
  env: true,
  gitignore: true,
  lock: true,
  log: true,
  text: true,
  tsv: true,
  txt: true,
};

/** Extension → Shiki grammar id, built from grammar ids and aliases on first use. */
let languageByExtension: Map<string, string> | null = null;

function resolveShikiLanguage(ext: string): string | undefined {
  if (!languageByExtension) {
    languageByExtension = new Map();
    for (const language of bundledLanguagesInfo) {
      languageByExtension.set(language.id, language.id);
      for (const alias of language.aliases ?? []) {
        languageByExtension.set(alias, language.id);
      }
    }
  }
  return languageByExtension.get(ext);
}

export function classifyFileView(
  filename: string,
  mime?: string,
): FileViewKind {
  const dot = filename.lastIndexOf(".");
  const ext =
    dot === -1 || dot === filename.length - 1
      ? null
      : filename.slice(dot + 1).toLowerCase();

  if (ext) {
    if (MARKDOWN_EXTENSIONS[ext]) return { kind: "markdown" };
    // Plain-text extensions win over Shiki: several (`log`, `csv`) exist as
    // grammars whose highlighting adds noise rather than meaning.
    if (TEXT_EXTENSIONS[ext]) return { kind: "text" };
    const language =
      EXTENSION_LANGUAGE_OVERRIDES[ext] ?? resolveShikiLanguage(ext);
    if (language) return { kind: "code", language };
  }

  // Extension unknown or absent: fall back to the imeta MIME. Generic
  // container types (octet-stream, pdf, zip…) stay non-viewable.
  const normalizedMime = mime?.split(";")[0].trim().toLowerCase();
  if (normalizedMime === "text/markdown") return { kind: "markdown" };
  if (normalizedMime === "application/json")
    return { kind: "code", language: "json" };
  if (normalizedMime?.startsWith("text/")) return { kind: "text" };

  return { kind: "none" };
}
