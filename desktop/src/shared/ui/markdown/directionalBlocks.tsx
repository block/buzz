import type * as React from "react";

import { cn } from "@/shared/lib/cn";

// The markdown blocks whose chrome flanks the text — list markers, the quote
// bar, and the inline padding that clears them — and so must carry `dir="auto"`
// rather than lean on the `unicode-bidi: plaintext` rule in
// styles/globals/markdown.css. That rule resolves each bidi paragraph's
// direction for the text itself, but leaves the `direction` property alone, and
// marker side plus `padding-inline-start` follow `direction`. Without the
// attribute an RTL message (Hebrew, Arabic, …) renders right-aligned text with
// its bullets and quote bar stranded on the far side of the block.

const ITEM_CLASS = "[&_p]:inline";
const LIST_CLASS = "space-y-1 ps-6 marker:text-muted-foreground/80";
const QUOTE_CLASS =
  "border-s-2 border-border ps-4 italic text-muted-foreground [&>*:first-child]:mt-0 [&>*+*]:mt-2";

export function MarkdownBlockquote({
  children,
}: {
  children?: React.ReactNode;
}) {
  return (
    <blockquote dir="auto" className={QUOTE_CLASS}>
      {children}
    </blockquote>
  );
}

// Deliberately no `dir` here: `dir="auto"` on the list skips any descendant
// that carries its own `dir`, so marking items too leaves the list with no
// strong character to read and it falls back to `ltr` — markers and indent
// stranded on the left. Items still get their own text direction from the
// `unicode-bidi: plaintext` rule; the list owns the marker side for all of them.
export function MarkdownListItem({ children }: { children?: React.ReactNode }) {
  return <li className={ITEM_CLASS}>{children}</li>;
}

export function MarkdownOrderedList({
  children,
}: {
  children?: React.ReactNode;
}) {
  return (
    <ol dir="auto" className={cn("list-decimal", LIST_CLASS)}>
      {children}
    </ol>
  );
}

export function MarkdownUnorderedList({
  children,
}: {
  children?: React.ReactNode;
}) {
  return (
    <ul dir="auto" className={cn("list-disc", LIST_CLASS)}>
      {children}
    </ul>
  );
}
