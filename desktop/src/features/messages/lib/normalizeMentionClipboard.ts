import {
  CHANNEL_LABEL_ATTRIBUTE,
  matchChipTextToLabel,
  MENTION_LABEL_ATTRIBUTE,
  MENTION_PUBKEY_ATTRIBUTE,
} from "./mentionClipboard";

/**
 * Detect whether clipboard HTML contains Buzz mention / channel-link
 * elements (marked with `data-mention` or `data-channel-link` attributes).
 */
export function hasMentionClipboardHtml(html: string): boolean {
  return html.includes("data-mention") || html.includes("data-channel-link");
}

/**
 * Put back the `@` / `#` a rendered chip strips for display.
 *
 * Exported for unit coverage: the surrounding normalization needs a DOM, this
 * decision doesn't.
 */
export function restoreChipSigil(text: string, sigil: "@" | "#"): string {
  if (!text || text.startsWith(sigil)) return text;
  return `${sigil}${text}`;
}

/**
 * Tags whose boundaries a reader sees as a line break.
 *
 * `innerText` derives this from layout, but a `DOMParser` document is never
 * rendered, so it falls back to `textContent` and runs "…the bug" straight into
 * "@John Smith". The visibility check that reads this text requires a boundary
 * before the sigil, so without the breaks a mention opening a paragraph would
 * look invisible and lose the identity it was copied with.
 */
const BLOCK_LEVEL_TAGS = new Set([
  "ADDRESS",
  "ARTICLE",
  "ASIDE",
  "BLOCKQUOTE",
  "BR",
  "DD",
  "DIV",
  "DL",
  "DT",
  "FIGCAPTION",
  "FIGURE",
  "FOOTER",
  "H1",
  "H2",
  "H3",
  "H4",
  "H5",
  "H6",
  "HEADER",
  "HR",
  "LI",
  "MAIN",
  "NAV",
  "OL",
  "P",
  "PRE",
  "SECTION",
  "TABLE",
  "TD",
  "TH",
  "TR",
  "UL",
]);

/**
 * Tags whose entire content ProseMirror's `DOMParser` drops on paste — its
 * `ignoreTags` list (prosemirror-model 1.25.4).
 *
 * The visibility gate reads the text returned here, so text under one of these
 * must not vouch for an identity record: `view.pasteHTML` never inserts it, and
 * a `<style>@Jane Doe</style>` beside an empty record span would otherwise
 * register a binding nobody can see. On a prosemirror-model upgrade, mirror
 * additions to its list — a tag PM ignores but we count reopens that bypass;
 * a tag we strip but PM inserts only over-drops a record, the safe direction.
 */
const PROSEMIRROR_IGNORED_TAGS_SELECTOR =
  "head, noscript, object, script, style, title";

/** The text `node`'s subtree contributes, with block boundaries as newlines. */
function readRenderedText(node: Node): string {
  let text = "";
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType === Node.TEXT_NODE) {
      text += child.nodeValue ?? "";
      continue;
    }
    if (!(child instanceof Element)) continue;
    const inner = readRenderedText(child);
    text += BLOCK_LEVEL_TAGS.has(child.tagName) ? `\n${inner}\n` : inner;
  }
  return text;
}

/** Clipboard HTML ready to insert, paired with the text it will contribute. */
export type MentionClipboardContent = {
  html: string;
  /**
   * What the reader will see. Both come from the same parse, so a caller
   * deciding what the paste made visible cannot be reading different markup
   * from the one being inserted.
   */
  text: string;
};

/**
 * Normalize clipboard HTML that contains Buzz mention / channel-link
 * elements.  Replaces the styled `<span data-mention>` and
 * `<button data-channel-link>` wrappers with unstyled text nodes so
 * TipTap's Bold extension doesn't misinterpret their font-weight as bold.
 *
 * Returns cleaned HTML that preserves surrounding formatting (bold, italic,
 * line breaks, etc.) while stripping only the mention/channel-link styling,
 * alongside that HTML's rendered text.
 */
export function normalizeMentionClipboardContent(
  html: string,
): MentionClipboardContent {
  const doc = new DOMParser().parseFromString(html, "text/html");

  // Drop what the paste will drop, before either output is derived — the pair
  // stays one view of what the composer ends up holding. Sweep the whole
  // document, not just `<body>`: the parser hoists a leading `<style>` or
  // `<title>` into `<head>`. Stripping ahead of the chip flattening below also
  // removes a `data-mention` span nested in `<noscript>`/`<object>` outright,
  // rather than flattening it into text the gate would then count as visible.
  for (const el of Array.from(
    doc.querySelectorAll(PROSEMIRROR_IGNORED_TAGS_SELECTOR),
  )) {
    el.remove();
  }

  for (const el of Array.from(
    doc.querySelectorAll("[data-mention], [data-channel-link]"),
  )) {
    // Replace the styled wrapper with a plain <span> containing the text —
    // and nothing else, so the identity attributes never reach the composer.
    // This preserves the text content inline while stripping the
    // font-weight/color styles that would confuse Tiptap's mark detection.
    const span = doc.createElement("span");
    const isMention = el.hasAttribute("data-mention");
    const sigil = isMention ? "@" : "#";
    const label = el.getAttribute(
      isMention ? MENTION_LABEL_ATTRIBUTE : CHANNEL_LABEL_ATTRIBUTE,
    );
    const text = el.textContent ?? "";
    // The rendered chip strips its sigil for display, so flattening it
    // verbatim would paste dead text that no composer can re-light. Restore
    // the sigil unless the source already carries it (Buzz's own copy
    // handlers write it back before the HTML reaches the clipboard) —
    // unless the chip is a fragment. Buzz's copy handlers decline a
    // selection whose only chip is partially covered, so the browser's
    // default copy serializes that chip with its full attributes around the
    // covered slice of its text. A sigil there would invent a mention the
    // user never copied ("@John" out of "John Smith"); the fragment stays
    // plain text, mirroring `restoreChipSigils` on the copy side, and with
    // no sigiled label in the inserted content the visibility gate discards
    // the identity record too.
    //
    // Every non-fragment match writes the *declared* label back, trimmed, the
    // way `restoreChipSigils` does on the copy side. `matchChipTextToLabel`
    // deliberately tolerates what a whole chip picks up in transit — a
    // pasteboard's U+00A0 for a space, padding, the author's casing, the
    // ellipsis the inline-chip cap renders — but every downstream matcher
    // (`getMentionOffsets`, and so the visibility gate, the decorations, and
    // the send-time extractor) requires the label's literal characters. Writing
    // the text verbatim would let a tolerance leak into the composer as sigiled
    // dead text whose identity record is then silently dropped; deriving it
    // from the same attribute the records carry keeps the two provably
    // consistent, whatever the classifier goes on to tolerate.
    const match =
      label === null
        ? "full"
        : matchChipTextToLabel(
            text,
            label,
            sigil,
            el.getAttribute(MENTION_PUBKEY_ATTRIBUTE) ?? undefined,
          );
    span.textContent =
      match === "fragment"
        ? text
        : restoreChipSigil(label === null ? text : label.trim(), sigil);
    el.replaceWith(span);
  }

  // Also strip any inline font-weight styles on remaining elements that
  // could be misinterpreted as bold by Tiptap (font-weight >= 500).
  for (const el of Array.from(doc.querySelectorAll("[style]"))) {
    if (el instanceof HTMLElement) {
      const fw = el.style.fontWeight;
      // Remove font-weight if it's the mention-highlight value (600)
      // but not an intentional bold (700/bold).
      if (fw === "600") {
        el.style.removeProperty("font-weight");
        if (!el.getAttribute("style")?.trim()) {
          el.removeAttribute("style");
        }
      }
    }
  }

  return { html: doc.body.innerHTML, text: readRenderedText(doc.body) };
}
