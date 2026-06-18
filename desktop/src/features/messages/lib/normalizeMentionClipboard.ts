/**
 * Detect whether clipboard HTML contains Buzz mention / channel-link
 * elements (marked with `data-mention` or `data-channel-link` attributes).
 */
export function hasMentionClipboardHtml(html: string): boolean {
  return html.includes("data-mention") || html.includes("data-channel-link");
}

/**
 * Normalize clipboard HTML that contains Buzz mention / channel-link
 * elements.  Replaces the styled `<span data-mention>` and
 * `<button data-channel-link>` wrappers with unstyled text nodes so
 * TipTap's Bold extension doesn't misinterpret their font-weight as bold.
 *
 * Returns cleaned HTML string that preserves surrounding formatting
 * (bold, italic, line breaks, etc.) while stripping only the mention/
 * channel-link styling.
 */
export function normalizeMentionClipboardHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");

  for (const el of Array.from(
    doc.querySelectorAll("[data-mention], [data-channel-link]"),
  )) {
    // Replace the styled wrapper with a plain <span> containing the text.
    // This preserves the text content inline while stripping the
    // font-weight/color styles that would confuse Tiptap's mark detection.
    const span = doc.createElement("span");
    span.textContent = el.textContent ?? "";
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

  return doc.body.innerHTML;
}
