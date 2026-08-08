/**
 * The extension set for vault notes.
 *
 * Shared by the live editor and the round-trip probe. They must agree: if the
 * probe measured a different schema than the editor uses, the guard would bless
 * files the editor then reformats.
 *
 * Kept deliberately close to CommonMark. Wikilinks are decoration-only — they
 * stay plain text in the document and so serialize back byte-identically.
 * Constructs that would need a real node (callouts, tables, footnotes) are
 * absent on purpose: until an extension can both render *and* serialize one,
 * the round-trip guard correctly routes those files to source mode rather than
 * silently eating them.
 */
import type { Extensions } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Table } from "@tiptap/extension-table";
import { TableCell } from "@tiptap/extension-table-cell";
import { TableHeader } from "@tiptap/extension-table-header";
import { TableRow } from "@tiptap/extension-table-row";
import { Markdown } from "tiptap-markdown";

import { ObsidianSyntaxExtension } from "@/features/documents/lib/editor/obsidianSyntaxExtension";
import { WikilinkExtension } from "@/features/documents/lib/editor/wikilinkExtension";

export function vaultEditorExtensions(): Extensions {
  return [
    StarterKit.configure({
      // Notes are documents, not chat: headings and horizontal rules are
      // wanted here, unlike in the message composer.
      codeBlock: {
        HTMLAttributes: { spellcheck: "false" },
      },
      code: {
        HTMLAttributes: { spellcheck: "false" },
      },
      // StarterKit's trailing-node plugin appends an empty paragraph after
      // block nodes. In a file-backed document that is a phantom blank line
      // that would be written to disk.
      trailingNode: false,
      // Configured separately below is unnecessary here — the default Link
      // behaviour is right for notes, but autolinking would rewrite bare URLs
      // the user typed as plain text, so it stays off.
      link: {
        autolink: false,
        openOnClick: false,
      },
    }),
    // Without these, a GFM table has no schema node: markdown-it still parses
    // one, the nodes are dropped, and the table serializes back as its bare
    // concatenated cell text. `tiptap-markdown` ships serialization for them.
    Table.configure({ resizable: false }),
    TableRow,
    TableHeader,
    TableCell,
    Markdown.configure({
      // Preserve the source as closely as the serializer allows.
      breaks: false,
      html: false,
      linkify: false,
      transformPastedText: false,
    }),
    // Both decoration-only: wikilinks, callouts, highlights, comments and tags
    // stay plain text in the document, so they serialize back byte-identically
    // and the round-trip guard still passes.
    WikilinkExtension,
    ObsidianSyntaxExtension,
  ];
}
