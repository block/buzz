/**
 * The extension set for vault notes.
 *
 * Shared by the live editor and the round-trip probe. They must agree: if the
 * probe measured a different schema than the editor uses, the guard would bless
 * files the editor then reformats.
 *
 * Kept deliberately close to CommonMark + GFM for now. Obsidian syntax
 * (wikilinks, callouts, highlights, tags) arrives in later phases; until an
 * extension can both render *and* serialize a construct, the round-trip guard
 * correctly routes those files to source mode rather than silently eating them.
 */
import type { Extensions } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Markdown } from "tiptap-markdown";

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
    Markdown.configure({
      // Preserve the source as closely as the serializer allows.
      breaks: false,
      html: false,
      linkify: false,
      transformPastedText: false,
    }),
  ];
}
